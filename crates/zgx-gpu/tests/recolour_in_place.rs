//! A fade's frame recolours runs of a paragraph the renderer keeps shaped:
//! its kept glyph quads take their runs' new colours in place, none placed
//! again from the atlas, and the frame is the one a renderer drawing it
//! afresh draws: glyphs, an underline following its run's colour, and a
//! code chip's wash.
use std::sync::Arc;
use zgx::{
    rich_text::{Decoration, RichText, TextRun},
    scene::*,
};
use zgx_gpu::GpuRenderer;

const W: u32 = 320;
const H: u32 = 160;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const TEXT: &str =
    "Words shown already, then a link and code() fading in, ÅÉÎ gypsy, then the newest words.";

fn paragraph(alpha: u8) -> NodeKind {
    let at = |s: &str| TEXT.find(s).unwrap();
    let (link, code, newest) = (at("a link"), at("code()"), at(", then the newest"));
    let run = |range: std::ops::Range<usize>, color: Color| TextRun {
        range,
        font_size: 18.,
        color,
        ..TextRun::default()
    };
    let fading = |c: Color| Color(c.0, c.1, c.2, alpha);
    let white = Color(255, 255, 255, 255);
    let accent = Color(120, 180, 255, 255);
    NodeKind::RichText {
        text: Arc::new(
            RichText::new(
                TEXT,
                vec![
                    run(0..link, white),
                    TextRun {
                        underline: Some(Decoration::new(1.5)),
                        ..run(link..code, fading(accent))
                    },
                    TextRun {
                        background: Some(Color(60, 60, 70, 255)),
                        padding: [3., 1.],
                        radius: 3.,
                        ..run(code..code + "code()".len(), fading(white))
                    },
                    run(code + "code()".len()..newest, fading(white)),
                    run(newest..TEXT.len(), fading(white)),
                ],
            )
            .unwrap(),
        ),
    }
}

fn scene_of(gpu: &GpuRenderer, alpha: u8) -> (Scene, NodeId) {
    let mut scene = Scene::new(W as f32, H as f32);
    gpu.install_text(&mut scene);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    scene.append(
        root,
        NodeKind::Rect(Color(16, 18, 24, 255)),
        Style {
            width: Some(W as f32),
            height: Some(H as f32),
            ..Style::default()
        },
    );
    let text = scene.append(
        root,
        paragraph(alpha),
        Style {
            width: Some(290.),
            text_wrap: true,
            ..Style::default()
        },
    );
    scene.set_transform(text, Transform { x: 12., y: 10. });
    (scene, text)
}

#[test]
fn a_recoloured_paragraph_keeps_its_glyph_quads_and_draws_as_afresh() {
    let _gpu_fixture = gpu_fixture();
    let mut gpu = GpuRenderer::new(W, H).expect("a Vulkan/Metal adapter is required");
    let (mut scene, text) = scene_of(&gpu, 20);
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    for alpha in [70, 140, 210, 255] {
        scene.set_leaf_kind(text, paragraph(alpha));
        let damage = scene.flush().damage;
        assert!(!damage.is_empty());
        let stats = gpu.render(&scene, &damage).unwrap();
        assert_eq!(stats.shaped_nodes, 0, "alpha {alpha}: shaped again");
        assert_eq!(
            stats.geometry_rebuilds, 0,
            "alpha {alpha}: its quads placed again from the atlas"
        );
        let drawn = gpu.readback().unwrap();
        // Drawn afresh by a renderer of its own, at this colour from the start.
        let mut fresh = GpuRenderer::new(W, H).unwrap();
        let (mut again, _) = scene_of(&fresh, alpha);
        let damage = again.flush().damage;
        fresh.render(&again, &damage).unwrap();
        assert!(
            drawn == fresh.readback().unwrap(),
            "alpha {alpha}: the recoloured frame differs from one drawn afresh"
        );
    }
}
