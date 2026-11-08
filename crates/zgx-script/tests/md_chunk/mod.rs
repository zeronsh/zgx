//! A streamed Markdown reply grown chunk by chunk, each phase's instructions
//! for a chunk counted (`zgx::profile::take`), shared by `md_chunk_flat`
//! (headless) and `md_chunk_flat_render` (drawn too).
use zgx::profile::Phase;
use zgx::scene::{Rect, Scene};
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

/// The script, its column in a scroll view the window's height that stays
/// at its end as it grows if `followed` (a chat following its reply): each
/// chunk then shows where it lands.
fn script(followed: bool) -> String {
    if !followed {
        return SCRIPT.into();
    }
    let open = "return ui.column {";
    assert!(SCRIPT.contains(open));
    let wrapped = SCRIPT.replacen(
        open,
        "return ui.scroll { follow = true, w = 640, h = 900, ui.column {",
        1,
    );
    format!("{}\n}}\n", wrapped.trim_end())
}

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
/// whole, then a section streamed uncounted), the reply `followed` or not
/// (`script`). `draw` gets each frame's scene
/// and damage after its flush, the whole reply's first.
pub fn per_chunk(
    sections: usize,
    measured: usize,
    followed: bool,
    mut draw: impl FnMut(&Scene, &[Rect]),
) -> Vec<(Phase, u64)> {
    let mut ui = Ui::new(640., 900.);
    let host = ScriptHost::inline(&ui.runtime, "md_chunk_flat", script(followed));
    host.reload().expect("the script loads");
    let _view = ui.mount(host.view());
    let chunks = sections * 3;
    let text: String = (1..=chunks).map(piece).collect();
    host.set_state("reply", Value::from(text.as_str())).unwrap();
    host.set_state("chunks", Value::Num(chunks as f64)).unwrap();
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    draw(&ui.scene.borrow(), &damage);
    let mut chunk = || {
        assert!(host.run_command("append"));
        ui.prepare_frame();
        let damage = ui.scene.borrow_mut().flush().damage;
        draw(&ui.scene.borrow(), &damage);
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

/// Each phase's cost a chunk at 100, 200, 400 and 800 sections, `followed`
/// or not, drawn by
/// `draw` (made afresh for each size): a phase that grows must be on
/// `still_growing`, with whose it is, and a listed one that went flat fails
/// until it is taken off.
pub fn check<D: FnMut(&Scene, &[Rect])>(
    still_growing: &[(Phase, &str)],
    followed: bool,
    mut draw: impl FnMut() -> D,
) {
    // SAFETY: each test using this runs alone in its process, before any
    // thread reads the environment.
    unsafe { std::env::set_var("ZGX_PROFILE", "1") };
    let sizes = [100, 200, 400, 800];
    let runs: Vec<Vec<(Phase, u64)>> = sizes
        .iter()
        .map(|&s| per_chunk(s, 9, followed, draw()))
        .collect();
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
        let listed = still_growing.iter().find(|(p, _)| p == phase);
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
