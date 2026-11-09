//! The shared scroll scenes, headless: their actual wheel path, with row-build
//! and phase instruction telemetry. Registered in perf/gate.py for cachegrind.
//! ZGX_SCROLL_STRICT=1 enforces the no-build-after-warmup contract; GPU=1 also
//! checks pixel-copy use and partial/full render equivalence on a real adapter.
#[path = "support/bench_limits.rs"]
mod bench_limits;
use zgx::profile::{Phase, span};
use zgx_script::{ScriptHost, testing::TestApp};

fn main() {
    let scene = std::env::var("SCENE").unwrap_or_else(|_| "transcript".into());
    let source = if scene == "transcript" || scene == "tables" {
        "transcript_scroll"
    } else {
        "scroll_scenes"
    };
    let examples = std::env::var("ZGX_EXAMPLES")
        .unwrap_or_else(|_| format!("{}/examples", env!("CARGO_MANIFEST_DIR")));
    let path = format!("{examples}/{source}/app.luau");
    let mut app = TestApp::with_desktop(|runtime| {
        ScriptHost::new(runtime, path)
            .limits(bench_limits::limits())
            .args([scene.clone()])
    });
    let scale: f32 = std::env::var("SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2.);
    app.ui.scene.borrow_mut().resize(960., 720.);
    app.ui.scene.borrow_mut().set_pixel_scale(scale);
    app.ui.set_assistive_technology(false);
    let mut gpu = std::env::var_os("GPU").map(|_| {
        let mut gpu = zgx_gpu::GpuRenderer::new((960. * scale) as u32, (720. * scale) as u32)
            .expect("GPU adapter");
        gpu.set_scale_factor(scale);
        gpu.debug_enable_single_pass();
        gpu.debug_allow_scroll_copies();
        gpu
    });
    zgx::trace::set_enabled(true);
    let mut draw = |app: &mut TestApp| {
        app.ui.prepare_frame();
        let damage = app.ui.scene.borrow_mut().flush().damage;
        if let Some(gpu) = &mut gpu {
            gpu.render(&app.ui.scene.borrow(), &damage)
                .expect("render scroll scene");
            let partial = gpu.debug_present_offscreen().expect("partial pixels");
            gpu.render(
                &app.ui.scene.borrow(),
                &[zgx::scene::Rect::new(0., 0., 960., 720.)],
            )
            .expect("full render");
            let full = gpu.debug_present_offscreen().expect("full pixels");
            assert!(partial == full, "stale paint after scroll");
        }
        app.frame();
    };
    app.frames(4);
    // Visit this exact region twice. Reversing must reuse rows built there,
    // and a third traversal measures only the warm path.
    for _ in 0..2 {
        for direction in [1., -1.] {
            for _ in 0..48 {
                app.scroll("#transcript", direction * 4.);
                draw(&mut app);
            }
        }
    }
    let from = zgx::trace::frame_index();
    zgx::profile::take();
    for direction in [1., -1.] {
        for _ in 0..48 {
            let span = span(Phase::Input);
            app.scroll("#transcript", direction * 4.);
            drop(span);
            draw(&mut app);
        }
    }
    let frames: Vec<_> = zgx::trace::frames(480)
        .into_iter()
        .filter(|f| f.index > from)
        .collect();
    let rows: usize = frames
        .iter()
        .map(|f| {
            f.notes
                .iter()
                .filter(|n| n.as_ref() == "list built row")
                .count()
        })
        .sum();
    let copied = frames
        .iter()
        .filter(|f| {
            f.notes
                .iter()
                .any(|n| n.contains("copied its pixels") || n.contains("composited its picture"))
        })
        .count();
    let max_instructions = frames
        .iter()
        .map(|f| {
            f.phases
                .iter()
                .filter(|p| p.0 != "wait")
                .map(|p| p.1)
                .sum::<u64>()
        })
        .max()
        .unwrap_or(0);
    assert!(app.error().is_none(), "script failed: {:?}", app.error());
    println!(
        "scroll scene={scene} frames={} rows_built={rows} copied={copied} max_instructions={max_instructions}",
        frames.len()
    );
    if std::env::var_os("ZGX_SCROLL_STRICT").is_some() {
        assert_eq!(rows, 0, "warm scroll rebuilt rows");
        if std::env::var_os("GPU").is_some() {
            assert!(
                copied >= 96,
                "tables/fades prevented scroll copies: {copied}/96"
            );
        }
        if max_instructions > 0 && std::env::var_os("GPU").is_none() {
            assert!(
                max_instructions <= 50_000_000,
                "scroll frame instruction ceiling"
            );
        }
    }
}
