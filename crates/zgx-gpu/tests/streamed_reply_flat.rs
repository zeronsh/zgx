//! A streamed reply's paragraph as Zeron's transcript shows it (`reply`:
//! chips, bold, a link; each piece fading in as a run of its own) costs the
//! same a frame however long it has grown: each phase's instructions for a
//! frame appending a piece, and for one fading pieces in (recolouring
//! alone), at 240 to 1920 bytes, laid out and drawn by a renderer of its
//! own (its damage only). Its text costs nothing more as it grows; its runs
//! still do, a little each (a frame's rich text is made, keyed and compared
//! run by run): a paragraph styled as densely throughout, its runs as many
//! as its bytes, is checked to a bound of its own. Its own process: it
//! counts with `ZGX_PROFILE` (`zgx::profile::take`).
use std::sync::Arc;
use zgx::{profile::Phase, scene::NodeKind};

#[path = "reply/mod.rs"]
mod reply;

const PHASES: [Phase; 5] = [
    Phase::Layout,
    Phase::Text,
    Phase::Damage,
    Phase::Paint,
    Phase::Encode,
];

/// Each phase's instructions for a frame appending a piece and for one
/// fading alone (the medians of 6 and 12), once `shown` bytes show.
fn per_frame(shown: usize, dense: bool) -> [Vec<u64>; 2] {
    let (width, height) = (860., 1300.);
    let mut gpu = zgx_gpu::GpuRenderer::new(width as u32, height as u32).expect("a GPU");
    let mut stream = match dense {
        true => reply::Reply::new(16),
        false => reply::Reply::prose(2400),
    };
    assert!(stream.text.len() > shown + 24 * 8, "room to stream");
    stream.shown_to(shown);
    let (mut scene, paragraph) = reply::window(&gpu, (width, height), stream.rich());
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    let mut frame = |step: usize| {
        zgx::profile::take();
        match step % 3 {
            0 => stream.append(),
            _ => stream.tick(),
        }
        scene.set_leaf_kind(
            paragraph,
            NodeKind::RichText {
                text: Arc::new(stream.rich()),
            },
        );
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        zgx::profile::take()
    };
    for step in 0..6 {
        frame(step);
    }
    let counted: Vec<_> = (6..24).map(|step| (step % 3 == 0, frame(step))).collect();
    [true, false].map(|appends| {
        PHASES
            .iter()
            .map(|phase| {
                let mut each: Vec<u64> = counted
                    .iter()
                    .filter(|(append, _)| *append == appends)
                    .map(|(_, c)| {
                        c.iter()
                            .find(|(p, ..)| p == phase)
                            .map_or(0, |(_, n, _)| *n)
                    })
                    .collect();
                each.sort_unstable();
                each[each.len() / 2]
            })
            .collect()
    })
}

/// The paragraph `dense` or not (`per_frame`), each frame's phases at 240
/// to 1920 bytes: the work of one (layout with its text, damage, paint) at
/// the longest at most `bound` times at 480 bytes. (Shorter, the paragraph
/// is fewer lines than a frame's damage reaches: it draws less than one
/// grown past them does, however long.)
fn check(dense: bool, bound: f64) {
    // SAFETY: each test runs alone in its process, before any thread reads
    // the environment.
    unsafe { std::env::set_var("ZGX_PROFILE", "1") };
    let sizes = [240, 480, 960, 1440, 1920];
    let runs: Vec<[Vec<u64>; 2]> = sizes.iter().map(|&n| per_frame(n, dense)).collect();
    if runs[0][0].iter().all(|n| *n == 0) {
        eprintln!("no instruction counter here: nothing to compare");
        return;
    }
    for (kind, name) in [(0, "appending a piece"), (1, "fading alone")] {
        eprintln!(
            "a frame {name} (dense: {dense}), thousands of instructions, at {sizes:?} bytes:"
        );
        for (i, phase) in PHASES.iter().enumerate() {
            let row: Vec<u64> = runs.iter().map(|r| r[kind][i] / 1000).collect();
            eprintln!("  {phase:?}: {row:?}");
        }
        let work = |run: &[Vec<u64>; 2]| run[kind][0] + run[kind][1] + run[kind][2] + run[kind][3];
        let (first, last) = (work(&runs[1]), work(&runs[runs.len() - 1]));
        eprintln!(
            "  layout, damage and paint, 1920 to 480 bytes: {:.2}x",
            last as f64 / first as f64
        );
        assert!(
            last as f64 <= first as f64 * bound,
            "a frame {name} at 1920 bytes costs {last} instructions, at 480 {first}"
        );
    }
}

/// Styled as Zeron's reply at its start, prose after: flat.
#[test]
fn a_streamed_reply_costs_the_same_a_frame_however_long() {
    check(false, 1.25);
}

/// Styled as densely throughout (its runs from 30 to 110): its runs' part
/// grows, a little a run (about 1,000 instructions a run a frame, made
/// optimized; several times that in a debug build).
#[test]
fn a_densely_styled_reply_costs_a_little_more_a_run() {
    check(true, 1.75);
}
