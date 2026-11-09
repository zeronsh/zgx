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

-- A state of the blocks, kept current as the reply grows.
local blocks = ui.markdown(reply, { key = "reply" })

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

/// Each phase's instructions for one chunk: the mean of `measured` chunks but
/// the dearest. A cost in one chunk of three is a cost, which a median hides;
/// one chunk's spike is noise, the most a loaded machine adds. Once the reply
/// holds `sections` (set whole, then a section streamed uncounted), the reply
/// `followed` or not (`script`). `draw` gets each frame's scene and damage
/// after its flush, the whole reply's first.
pub fn per_chunk(
    start: &str,
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
    let text = start.to_string() + &(1..=chunks).map(piece).collect::<String>();
    host.set_state("reply", Value::from(text.as_str())).unwrap();
    host.set_state("chunks", Value::Num(chunks as f64)).unwrap();
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    draw(&ui.scene.borrow(), &damage);
    let mut chunk = || {
        // Luau's collector run whole before each chunk, uncounted: a chunk
        // then does the collecting its own allocations bring on, the same
        // at every length, where a cycle's steps landing in some chunks and
        // not others (more of them the larger the heap) moved the count.
        host.collect_luau_only();
        zgx::profile::take();
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
            each.pop();
            (
                counted[0][i].0,
                each.iter().sum::<u64>() / each.len() as u64,
            )
        })
        .collect()
}

/// A phase flat as the reply grows costs at most this much more a chunk at
/// the longest reply than at the shortest.
const FLAT: f64 = 1.25;
/// A phase listed as growing must still grow by more than this to stay
/// listed: between it and `FLAT` either verdict stands, so a phase near the
/// line does not flip from run to run, while one gone flat is taken off and
/// one growing unlisted fails.
const STILL_GROWS: f64 = 1.10;
/// How many times a failing verdict is measured again.
const AGAIN: usize = 2;

/// Each phase's cost a chunk at 100, 200, 400 and 800 sections, `followed`
/// or not, drawn by
/// `draw` (made afresh for each size): a phase that grows must be on
/// `still_growing`, with whose it is, and a listed one that went flat fails
/// until it is taken off.
pub fn check<D: FnMut(&Scene, &[Rect])>(
    start: &str,
    still_growing: &[(Phase, &str)],
    followed: bool,
    mut draw: impl FnMut() -> D,
) {
    // SAFETY: each test using this runs alone in its process, before any
    // thread reads the environment.
    unsafe { std::env::set_var("ZGX_PROFILE", "1") };
    let sizes = [100, 200, 400, 800];
    let mut measure = || -> Vec<Vec<(Phase, u64)>> {
        sizes
            .iter()
            .map(|&s| per_chunk(start, s, 9, followed, draw()))
            .collect()
    };
    let mut runs = measure();
    if runs[0].iter().all(|(_, n)| *n == 0) {
        eprintln!("no instruction counter here: nothing to compare");
        return;
    }
    // Noise only adds instructions, and on a loaded machine can add a tenth
    // of a phase to one size's count, once. A verdict about to fail is
    // measured again, the lowest of each kept: real growth reads the same
    // every time.
    let mut wrong = wrong_phases(&runs, still_growing);
    let first_read = wrong.clone();
    for _ in 0..AGAIN {
        if wrong.is_empty() {
            break;
        }
        for (run, again) in runs.iter_mut().zip(measure()) {
            for (kept, (_, n)) in run.iter_mut().zip(again) {
                kept.1 = kept.1.min(n);
            }
        }
        wrong = wrong_phases(&runs, still_growing);
    }
    // Shown, so a cost that comes and goes isn't lost to the lowest count.
    if !first_read.is_empty() && wrong.is_empty() {
        eprintln!("rescued by measuring again; the first read: {first_read:?}");
    }
    eprintln!("per chunk, thousands of instructions, at {sizes:?} sections, after {start:?}:");
    for (i, (phase, _)) in runs[0].iter().enumerate() {
        let thousands: Vec<u64> = runs.iter().map(|r| r[i].1 / 1000).collect();
        let (grows, listed) = verdict(&runs, i, still_growing);
        eprintln!(
            "  {phase:?}: {thousands:?}{}",
            match (grows, listed) {
                (true, Some(whose)) => format!(" grows ({whose})"),
                (true, None) => " GROWS".into(),
                (false, Some(_)) => " flat now: take it off STILL_GROWING".into(),
                (false, None) => String::new(),
            }
        );
    }
    assert!(
        wrong.is_empty(),
        "phases not as STILL_GROWING says: {wrong:?}"
    );
}

/// Whether phase `i` grows from the shortest reply to the longest (by the
/// line for whether it's listed), and whose it is if listed.
fn verdict<'a>(
    runs: &[Vec<(Phase, u64)>],
    i: usize,
    still_growing: &'a [(Phase, &str)],
) -> (bool, Option<&'a str>) {
    let phase = runs[0][i].0;
    let (first, last) = (runs[0][i].1, runs[runs.len() - 1][i].1);
    let listed = still_growing.iter().find(|(p, _)| *p == phase);
    let by = if listed.is_some() { STILL_GROWS } else { FLAT };
    // (Growing by a few thousand instructions a chunk is noise, not growth:
    // the driver's submit, of about 23K, reads 32K now and then.)
    let grows = last > first + 20_000 && last as f64 > first as f64 * by;
    (grows, listed.map(|(_, whose)| *whose))
}

/// The phases not as `still_growing` says, with their counts.
fn wrong_phases(runs: &[Vec<(Phase, u64)>], still_growing: &[(Phase, &str)]) -> Vec<String> {
    (0..runs[0].len())
        .filter_map(|i| {
            let (grows, listed) = verdict(runs, i, still_growing);
            (grows != listed.is_some()).then(|| {
                let thousands: Vec<u64> = runs.iter().map(|r| r[i].1 / 1000).collect();
                format!("{:?} {thousands:?}", runs[0][i].0)
            })
        })
        .collect()
}
