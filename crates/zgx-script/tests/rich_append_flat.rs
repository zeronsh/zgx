//! A paragraph growing at its end (a streamed reply's last paragraph, 24
//! bytes a frame) costs what an append costs, however long it has grown:
//! each phase's instructions for a frame, laid out and drawn by a renderer
//! of its own (`GpuRenderer::render`, its damage only), at 240 to 1920
//! characters side by side. Its own process: it counts with `ZGX_PROFILE`
//! (`zgx::profile::take`).
use zgx::profile::Phase;
use zgx::widgets::Ui;
use zgx_script::{ScriptHost, Value};

const SCRIPT: &str = r###"
local reply = ui.state("reply", "")
local n = 0
ui.command { id = "append", run = function()
	n += 1
	reply:append(piece(n))
end }
function piece(n: number): string
	return if n % 2 == 1 then "Words arriving as reply " else "streams, with the rest. "
end
return ui.column {
	w = 684,
	PARAGRAPH,
}
"###;

/// The paragraph, its run listed and following the reply's state, and made
/// by a function (`spans`) reading it: kept in place either way.
const PARAGRAPHS: [&str; 2] = [
    "ui.rich { wrap = true, ui.span { reply } }",
    "ui.rich { wrap = true, spans = function() return { ui.span { reply:get(), literal = true } } end }",
];

/// The script's append `n` (from 1), 24 bytes each.
fn piece(n: usize) -> &'static str {
    if n % 2 == 1 {
        "Words arriving as reply "
    } else {
        "streams, with the rest. "
    }
}

/// The phases a frame of an append is counted in.
const PHASES: [Phase; 5] = [
    Phase::Layout,
    Phase::Text,
    Phase::Damage,
    Phase::Paint,
    Phase::Encode,
];

/// Each of `PHASES`' instructions for an append, the median of 9, once the
/// paragraph holds `chars` (set whole, then a few appends uncounted).
fn per_append(paragraph: &str, chars: usize) -> Vec<u64> {
    let mut ui = Ui::new(720., 900.);
    let mut gpu = zgx_gpu::GpuRenderer::new(720, 900).expect("a GPU");
    // Measured as a window measures it: from the text the renderer draws.
    gpu.install_text(&mut ui.scene.borrow_mut());
    let host = ScriptHost::inline(
        &ui.runtime,
        "rich_append_flat",
        SCRIPT.replace("PARAGRAPH", paragraph),
    );
    host.reload().expect("the script loads");
    let _view = ui.mount(host.view());
    let appends = chars / 24;
    let text: String = (1..=appends).map(piece).collect();
    // The script's count goes on from there: its pieces alternate.
    host.set_state("reply", Value::from(text.as_str())).unwrap();
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    gpu.render(&ui.scene.borrow(), &damage).unwrap();
    let mut append = || {
        host.collect_luau_only();
        zgx::profile::take();
        assert!(host.run_command("append"));
        ui.prepare_frame();
        let damage = ui.scene.borrow_mut().flush().damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        zgx::profile::take()
    };
    for _ in 0..3 {
        append();
    }
    let counted: Vec<_> = (0..9).map(|_| append()).collect();
    PHASES
        .iter()
        .map(|phase| {
            let mut each: Vec<u64> = counted
                .iter()
                .map(|c| {
                    c.iter()
                        .find(|(p, ..)| p == phase)
                        .map_or(0, |(_, n, _)| *n)
                })
                .collect();
            each.sort_unstable();
            each[each.len() / 2]
        })
        .collect()
}

#[test]
fn an_append_costs_the_same_however_long_the_paragraph() {
    // SAFETY: this test runs alone in its process, before any thread reads
    // the environment.
    unsafe { std::env::set_var("ZGX_PROFILE", "1") };
    let sizes = [240, 480, 960, 1440, 1920];
    for paragraph in PARAGRAPHS {
        let runs: Vec<Vec<u64>> = sizes.iter().map(|&n| per_append(paragraph, n)).collect();
        if runs[0].iter().all(|n| *n == 0) {
            eprintln!("no instruction counter here: nothing to compare");
            return;
        }
        eprintln!("{paragraph}: per append, thousands of instructions, at {sizes:?} characters:");
        for (i, phase) in PHASES.iter().enumerate() {
            let row: Vec<u64> = runs.iter().map(|r| r[i] / 1000).collect();
            eprintln!("  {phase:?}: {row:?}");
        }
        // Layout (with its text), damage and paint, together: flat as it
        // grows. (Before each append kept the lines before its last, a frame
        // at 1920 characters cost 4.3 times one at 240.)
        let work = |run: &Vec<u64>| run[0] + run[1] + run[2] + run[3];
        let (first, last) = (work(&runs[0]), work(&runs[runs.len() - 1]));
        assert!(
            last as f64 <= first as f64 * 1.25,
            "{paragraph}: an append at 1920 characters costs {last} instructions, at 240 {first}"
        );
    }
}
