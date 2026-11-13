//! The software raster draws plain text as the GPU renderer does at 1x,
//! sharing the font system and text cache the scene is measured with
//! (`Raster::share_text`): the same layout, the same glyph images on the
//! same whole pixels, the same coverage firming. Its frames match the GPU's
//! to within blending's rounding, light text on dark and dark on light, at
//! several sizes, wrapped, centred and scrolled. Without it, the raster
//! drew its bundled font a character at a time and wrapped lines where
//! layout hadn't (a word split across two lines).
use zgx::{compose::prelude::*, reactive::Signal, widgets::Ui};
use zgx_desktop::raster::Raster;
use zgx_gpu::GpuRenderer;

const W: u32 = 360;
const H: u32 = 240;

/// Words in rows at several sizes, a wrapped paragraph and a centred line,
/// in `ink` on `ground`.
fn page(ink: u32, ground: u32) -> View {
    let mut page = column().w(W as f32).h(H as f32).gap(4.).bg(rgb(ground));
    for (i, size) in [11., 12., 13., 14., 16., 20., 24.].into_iter().enumerate() {
        page = page.child(
            text(format!("{size}px: The quick brown fox jumps, row {i}"))
                .text_size(size)
                .text_color(rgb(ink)),
        );
    }
    page.child(
        text(
            "A paragraph long enough to wrap over several lines at this width, \
             so the raster's lines break where layout broke them.",
        )
        .w(W as f32 - 40.)
        .text_size(13.)
        .text_color(rgb(ink)),
    )
    .child(
        row()
            .w(W as f32)
            .justify_center()
            .child(text("centred").text_size(15.).text_color(rgb(ink))),
    )
}

/// The worst channel difference of each pixel, the raster's frame against
/// the GPU's.
fn differences(raster: &Raster, gpu: &GpuRenderer) -> Vec<u8> {
    let rgba = gpu.readback().expect("readback");
    raster
        .pixels
        .iter()
        .zip(rgba.as_chunks::<4>().0)
        .map(|(p, g)| {
            let r = [(p >> 16) as u8, (p >> 8) as u8, *p as u8];
            (0..3).map(|c| r[c].abs_diff(g[c])).max().unwrap()
        })
        .collect()
}

/// Whether `diffs` is within blending's rounding: no channel more than 4
/// of 255 off. Metal's frames differ by at most 1 (8-bit rounding of the
/// same blend); 4 leaves room for another backend's rounding, and is far
/// under what a glyph moved or drawn differently costs (the unshared
/// raster's worst is over 200, on a fifth of the pixels).
fn matches(diffs: &[u8]) -> Result<(), String> {
    let worst = diffs.iter().copied().max().unwrap_or(0);
    if worst > 4 {
        let off = diffs.iter().filter(|d| **d > 4).count();
        return Err(format!("worst {worst}, {off} pixels more than 4 off"));
    }
    Ok(())
}

/// A GPU renderer and a raster drawing `ui`'s scene, the raster sharing the
/// renderer's text (or its own bundled font, unshared). None without an
/// adapter.
fn renderers(ui: &Ui, shared: bool) -> Option<(GpuRenderer, Raster)> {
    let Ok(mut gpu) = GpuRenderer::new(W, H) else {
        eprintln!("no GPU adapter");
        return None;
    };
    gpu.set_background(zgx::scene::Color(0, 0, 0, 255));
    gpu.install_text(&mut ui.scene.borrow_mut());
    let mut raster = Raster::new(W as usize, H as usize);
    if shared {
        raster.share_text(gpu.text_system(), gpu.text_cache());
    }
    Some((gpu, raster))
}

/// Draws `ui`'s next frame both ways from its damage.
fn frame(ui: &mut Ui, gpu: &mut GpuRenderer, raster: &mut Raster) {
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    gpu.render(&ui.scene.borrow(), &damage).expect("render");
    raster.render(&ui.scene.borrow(), &damage);
}

#[test]
fn plain_text_matches_the_gpu_light_on_dark_and_dark_on_light() {
    for (ink, ground, label) in [
        (0xe0e6ef, 0x101820, "light on dark"),
        (0x1a1a1a, 0xfafafa, "dark on light"),
        (0x3a8adf, 0x202020, "colour on dark"),
    ] {
        let mut ui = Ui::new(W as f32, H as f32);
        ui.mount(page(ink, ground));
        let Some((mut gpu, mut raster)) = renderers(&ui, true) else {
            return;
        };
        frame(&mut ui, &mut gpu, &mut raster);
        matches(&differences(&raster, &gpu)).unwrap_or_else(|e| panic!("{label}: {e}"));
    }
}

/// The comparison bites: the raster's own font, unshared, is far off.
#[test]
fn unshared_text_does_not_match_the_gpu() {
    let mut ui = Ui::new(W as f32, H as f32);
    ui.mount(page(0xe0e6ef, 0x101820));
    let Some((mut gpu, mut raster)) = renderers(&ui, false) else {
        return;
    };
    frame(&mut ui, &mut gpu, &mut raster);
    assert!(matches(&differences(&raster, &gpu)).is_err());
}

/// Scrolled in steps (whole rows and strips between), each frame drawn from
/// its damage, the raster moving what scrolled: every frame still matches.
#[test]
fn scrolled_text_matches_the_gpu_frame_by_frame() {
    let mut ui = Ui::new(W as f32, H as f32);
    let offset: Signal<f32> = ui.signal(0.);
    let mut rows = column().w(W as f32).gap(3.);
    for i in 0..80 {
        rows = rows.child(
            text(format!("Row {i}: a line of text as a transcript shows it"))
                .text_size(13.)
                .text_color(rgb(0xe0e6ef)),
        );
    }
    ui.mount(
        scroll(offset.clone())
            .scrollbar(false)
            .w(W as f32)
            .h(H as f32)
            .bg(rgb(0x101820))
            .child(rows),
    );
    let Some((mut gpu, mut raster)) = renderers(&ui, true) else {
        return;
    };
    frame(&mut ui, &mut gpu, &mut raster);
    for step in [19., 7., 40., 1., -13., 60.] {
        offset.update(|o| *o += step);
        frame(&mut ui, &mut gpu, &mut raster);
        matches(&differences(&raster, &gpu)).unwrap_or_else(|e| panic!("after {step:+}: {e}"));
    }
    assert!(raster.scroll_copies() > 0, "no move copied");
}

/// Underlined plain text: each line's underline where the GPU's quad
/// covers pixels, whole or between two rows.
#[test]
fn underlined_text_matches_the_gpu() {
    use zgx::{
        scene::{Color, NodeKind, Scene, Style},
        text_layout::{FontStyle, TextUnderline},
    };
    let Ok(mut gpu) = GpuRenderer::new(W, H) else {
        eprintln!("no GPU adapter");
        return;
    };
    gpu.set_background(Color(0x10, 0x14, 0x1c, 255));
    let mut scene = Scene::new(W as f32, H as f32);
    gpu.install_text(&mut scene);
    for offset in [None, Some(2.), Some(2.5), Some(3.25)] {
        let id = scene.append(
            scene.root(),
            NodeKind::Text {
                text: "Underlined words, wrapping onto a second line here".into(),
                font_size: 14.,
                color: Color(224, 230, 239, 255),
            },
            Style {
                width: Some(220.),
                height: Some(50.),
                ..Default::default()
            },
        );
        scene.set_font(
            id,
            FontStyle {
                underline: Some(TextUnderline::new(offset)),
                ..Default::default()
            },
        );
    }
    let mut raster = Raster::new(W as usize, H as usize);
    raster.share_text(gpu.text_system(), gpu.text_cache());
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).expect("render");
    raster.render(&scene, &damage);
    // The raster's ground is its own; the GPU's clear is set to match it.
    matches(&differences(&raster, &gpu)).unwrap();
}

/// Text shared with its scene's services is trimmed after each raster
/// frame, as the GPU renderer's frames trim theirs: a value changing every
/// frame keeps the word cache the size of what is shown. Untrimmed, each
/// frame's new number stayed cached (0.7 MB a second at 60 frames).
#[test]
fn a_changing_value_keeps_the_shared_word_cache_the_size_of_what_is_shown() {
    use zgx::scene::{Color, NodeKind, Scene, Style};
    let services = zgx_gpu::TextServices::new();
    let mut scene = Scene::new(400., 60.);
    services.install(&mut scene);
    let label = scene.append(
        scene.root(),
        NodeKind::Text {
            text: "Requests 0 per second".into(),
            color: Color(255, 255, 255, 255),
            font_size: 16.,
        },
        Style::default(),
    );
    let mut raster = Raster::new(400, 60);
    raster.share_text(services.text_system(), services.text_cache());
    let mut most = 0;
    for frame in 1..400 {
        scene.set_text(label, format!("Requests {} per second", frame * 7919));
        let damage = scene.flush().damage;
        raster.render(&scene, &damage);
        most = most.max(services.text_system().borrow().shape_run_cache.len());
    }
    // "Requests", "per", "second", spaces and the last few values.
    assert!(most < 40, "{most} words cached for one changing label");
    assert!(services.text_system().borrow().shape_run_cache.len() >= 4);
}
