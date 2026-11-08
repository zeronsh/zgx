//! One chunk of a streamed Markdown reply costs what a chunk costs, however
//! long the reply has grown: each phase's instructions for a chunk at 100,
//! 200 and 400 sections, side by side (a `ui.markdown { key }` stream drawn
//! by `ui.each`, as `md_stream_bench` and the docs do it). Its own process:
//! it counts with `ZGX_PROFILE` (`zgx::profile::take`).
use zgx::profile::Phase;
use zgx::widgets::Ui;
use zgx_script::{ScriptHost, Value};

const SCRIPT: &str = r###"
local reply = ui.state("reply", "")
local chunks = ui.state("chunks", 0)

local function runs(list: { MarkdownRun }): Node
	local props: RichProps = { wrap = true }
	for _, r in list do
		table.insert(props, ui.span { r.x, literal = true, weight = if r.b then 600 else nil })
	end
	return ui.rich(props)
end

local blocks = ui.computed(function()
	return ui.markdown(reply:get(), { key = "reply" })
end)

ui.command { id = "append", run = function()
	local n = chunks:get() + 1
	chunks:set(n)
	reply:append(piece(n))
end }

function piece(n: number): string
	return if n % 3 == 1 then "## Section " .. ((n + 2) // 3) .. "\n\n"
		elseif n % 3 == 2 then "Words arriving as a reply streams, with **bold** and `code` in them, "
		else "and the rest of the paragraph after it.\n\n"
end

return ui.column {
	gap = 8,
	w = 600,
	ui.each(blocks, function(b: MarkdownBlock)
		if b.t == "p" or b.t == "h" then
			return runs(b.s)
		end
		return ui.div {}
	end),
}
"###;

/// The script's chunk `n` (from 1), as its `append` makes it.
fn piece(n: usize) -> String {
    match n % 3 {
        1 => format!("## Section {}\n\n", n.div_ceil(3)),
        2 => "Words arriving as a reply streams, with **bold** and `code` in them, ".into(),
        _ => "and the rest of the paragraph after it.\n\n".into(),
    }
}

/// Each phase's instructions for one chunk, the median of `measured`
/// chunks (the collector runs in some), once the reply holds `sections` (set
/// whole, then a section streamed uncounted).
fn per_chunk(sections: usize, measured: usize) -> Vec<(Phase, u64)> {
    let mut ui = Ui::new(640., 900.);
    let host = ScriptHost::inline(&ui.runtime, "md_chunk_flat", SCRIPT);
    host.reload().expect("the script loads");
    let _view = ui.mount(host.view());
    let chunks = sections * 3;
    let text: String = (1..=chunks).map(piece).collect();
    host.set_state("reply", Value::from(text.as_str())).unwrap();
    host.set_state("chunks", Value::Num(chunks as f64)).unwrap();
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let chunk = || {
        assert!(host.run_command("append"));
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        zgx::profile::take()
    };
    for _ in 0..3 {
        chunk();
    }
    let counted: Vec<Vec<(Phase, u64, std::time::Duration)>> =
        (0..measured).map(|_| chunk()).collect();
    (0..counted[0].len())
        .map(|i| {
            let mut each: Vec<u64> = counted.iter().map(|c| c[i].1).collect();
            each.sort_unstable();
            (counted[0][i].0, each[each.len() / 2])
        })
        .collect()
}

/// A phase flat as the reply grows costs at most this much more a chunk at
/// the longest reply than at the shortest.
const FLAT: f64 = 1.25;

/// Phases a chunk still costs more in as the reply grows, and whose: each
/// goes from here once it is flat (the test then fails until it does), and
/// any phase not here growing fails it.
const STILL_GROWING: &[(Phase, &str)] = &[
    // The parse's whole block list cloned and its table derived (flush lane).
    (Phase::Script, "flush"),
    // ui.each's and keyed's prefix scans, the rows' identities (flush lane);
    // damage marked across every block (render lane).
    (Phase::Update, "flush, render"),
    // The column measured and arranged whole as its last block grows
    // (layout lane).
    (Phase::Layout, "layout"),
];

#[test]
fn a_chunk_costs_the_same_however_long_the_reply() {
    // SAFETY: this test's process runs this test alone, before any thread
    // reads the environment.
    unsafe { std::env::set_var("ZGX_PROFILE", "1") };
    let sizes = [100, 200, 400, 800];
    let runs: Vec<Vec<(Phase, u64)>> = sizes.iter().map(|&s| per_chunk(s, 9)).collect();
    if runs[0].iter().all(|(_, n)| *n == 0) {
        eprintln!("no instruction counter here: nothing to compare");
        return;
    }
    eprintln!("per chunk, thousands of instructions, at {sizes:?} sections:");
    let mut wrong = Vec::new();
    for (i, (phase, _)) in runs[0].iter().enumerate() {
        let row: Vec<u64> = runs.iter().map(|r| r[i].1).collect();
        let (first, last) = (row[0], row[row.len() - 1]);
        // (Phases of a few thousand instructions: noise, not growth.)
        let grows = last > 20_000 && last as f64 > first as f64 * FLAT;
        let listed = STILL_GROWING.iter().find(|(p, _)| p == phase);
        let thousands: Vec<u64> = row.iter().map(|n| n / 1000).collect();
        eprintln!(
            "  {phase:?}: {thousands:?}{}",
            match (grows, listed) {
                (true, Some((_, whose))) => format!(" grows ({whose})"),
                (true, None) => " GROWS".into(),
                (false, Some(_)) => " flat now: take it off STILL_GROWING".into(),
                (false, None) => String::new(),
            }
        );
        if grows != listed.is_some() {
            wrong.push(*phase);
        }
    }
    assert!(
        wrong.is_empty(),
        "phases not as STILL_GROWING says: {wrong:?}"
    );
}
