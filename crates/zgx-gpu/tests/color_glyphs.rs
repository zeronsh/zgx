//! Colour glyphs (emoji) draw in their own colours whatever the text's
//! colour: white, grey, black and red text show the same emoji, their
//! shadows are their alphas in the shadow's colour, and selected, they keep
//! their colours under a selected-text colour.
use zgx::{
    compose::prelude::*,
    scene::{Color, NodeId},
    style::rgb,
    widgets::Ui,
};
use zgx_gpu::GpuRenderer;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 120;
const EMOJI: &str = "🌚😀👍🏽❤️🇯🇵👩‍💻";

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether this machine has a colour emoji font: macOS and Windows always
/// do; a Linux may not, and then draws outlines in the text's colour.
fn color_font() -> bool {
    let fonts = cosmic_text::FontSystem::new();
    let found = fonts.db().faces().any(|face| {
        face.families.iter().any(|(name, _)| {
            ["Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji"].contains(&name.as_str())
        })
    });
    assert!(
        found || !cfg!(any(target_os = "macos", windows)),
        "no colour emoji font"
    );
    found
}

/// `EMOJI` in `color` on a transparent frame, `line` styling its text,
/// rendered (checked against a full repaint) and read back.
fn draw(color: Color, line: impl FnOnce(View) -> View, select: bool) -> Vec<u8> {
    let mut gpu = GpuRenderer::new(WIDTH, HEIGHT).unwrap();
    let mut ui = Ui::new(WIDTH as f32, HEIGHT as f32);
    gpu.install_text(&mut ui.scene.borrow_mut());
    ui.refresh_text_geometry();
    ui.mount(
        column()
            .size(WIDTH as f32, HEIGHT as f32)
            .p(10.)
            .text_color(color)
            .text_size(32.)
            .selectable(true)
            .selected_text_color(Color(255, 0, 0, 255))
            .selection_color(Color(0, 0, 0, 0))
            .child(line(text(EMOJI))),
    );
    if select {
        let node: NodeId = {
            let scene = ui.scene.borrow();
            let column = scene.children(scene.root())[0];
            scene.children(column)[0]
        };
        assert!(ui.select_text(
            zgx::selection::TextPoint { node, offset: 0 },
            zgx::selection::TextPoint {
                node,
                offset: EMOJI.len(),
            },
        ));
    }
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    let scene = ui.scene.borrow();
    gpu.render(&scene, &damage).unwrap();
    let pixels = gpu.readback().unwrap();
    gpu.render(
        &scene,
        &[zgx::scene::Rect::new(0., 0., WIDTH as f32, HEIGHT as f32)],
    )
    .unwrap();
    assert!(
        pixels == gpu.readback().unwrap(),
        "incremental frame differs"
    );
    pixels
}

/// Whether white text's pixel `p` is of a colour glyph: its channels
/// apart. (A sequence the font lacks falls back to a plain glyph, drawn in
/// the text's colour as it should be: grey under white text. Linux's Noto
/// Color Emoji lacks some.)
fn in_colour(p: &[u8; 4]) -> bool {
    p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) > 8
}

/// How many lit pixels are coloured (their channels apart), and how many
/// are lit at all.
fn colors(pixels: &[u8]) -> (usize, usize) {
    let lit: Vec<&[u8; 4]> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] > 128)
        .collect();
    let colored = lit
        .iter()
        .filter(|p| {
            let (max, min) = (p[0].max(p[1]).max(p[2]), p[0].min(p[1]).min(p[2]));
            max - min > 60
        })
        .count();
    (colored, lit.len())
}

#[test]
fn emoji_keep_their_colours_whatever_the_text_colour() {
    let _gpu = gpu_fixture();
    if !color_font() {
        return;
    }
    let white = draw(Color(255, 255, 255, 255), |v| v, false);
    let (colored, lit) = colors(&white);
    assert!(
        lit > 1000 && colored > lit / 4,
        "in colour: {colored} of {lit}"
    );
    for color in [rgb(0x888888), rgb(0x000000), rgb(0xff0000)] {
        let other = draw(color, |v| v, false);
        let differ: Vec<(usize, [u8; 4], [u8; 4])> = (white.as_chunks::<4>().0.iter())
            .zip(other.as_chunks::<4>().0)
            .enumerate()
            .filter(|(_, (a, b))| in_colour(a) && a != b)
            .map(|(i, (a, b))| (i, *a, *b))
            .collect();
        assert!(
            differ.is_empty(),
            "{color:?} text draws its emoji as white text does: {} of {} pixels differ, \
             e.g. {:?}",
            differ.len(),
            colors(&white).1,
            differ
                .iter()
                .step_by(differ.len().div_ceil(6).max(1))
                .collect::<Vec<_>>()
        );
    }
    // Faded with its text: half the alpha, the same colours.
    // Where white text's emoji are opaque, half as opaque, and as coloured
    // (premultiplied: each channel about half).
    let faded = draw(Color(255, 255, 255, 128), |v| v, false);
    let (mut opaque, mut halved) = (0, 0);
    for (w, f) in white
        .as_chunks::<4>()
        .0
        .iter()
        .zip(faded.as_chunks::<4>().0)
    {
        if w[3] == 255 {
            opaque += 1;
            let half = |a: u8, b: u8| (i32::from(a) - 2 * i32::from(b)).abs() <= 4;
            if (0..4).all(|k| half(w[k], f[k])) {
                halved += 1;
            }
        }
    }
    assert!(
        opaque > 1000 && halved > opaque * 9 / 10,
        "faded: {halved} of {opaque} halved"
    );
}

#[test]
fn an_emoji_shadow_is_its_alpha_in_the_shadow_colour() {
    let _gpu = gpu_fixture();
    if !color_font() {
        return;
    }
    let plain = draw(Color(255, 255, 255, 255), |v| v, false);
    let shadowed = draw(
        Color(255, 255, 255, 255),
        |v| v.text_shadow(0., 50., 0., Color(0, 0, 255, 255)),
        false,
    );
    // Its shadow, below it: blue alone, where the emoji are coloured.
    let below = |pixels: &[u8]| {
        pixels.as_chunks::<4>().0[(70 * WIDTH) as usize..]
            .iter()
            .filter(|p| p[3] > 200)
            .map(|p| [p[0], p[1], p[2]])
            .collect::<Vec<_>>()
    };
    assert!(below(&plain).is_empty(), "nothing below without a shadow");
    let shadow = below(&shadowed);
    assert!(shadow.len() > 500, "a shadow: {}", shadow.len());
    assert!(
        shadow.iter().all(|p| p[0] < 10 && p[1] < 10 && p[2] > 200),
        "blue alone: {:?}",
        shadow
            .iter()
            .find(|p| p[0] >= 10 || p[1] >= 10 || p[2] <= 200)
    );
}

#[test]
fn selected_emoji_keep_their_colours() {
    let _gpu = gpu_fixture();
    if !color_font() {
        return;
    }
    let plain = draw(Color(255, 255, 255, 255), |v| v, false);
    let selected = draw(Color(255, 255, 255, 255), |v| v, true);
    let differ = (plain.as_chunks::<4>().0.iter())
        .zip(selected.as_chunks::<4>().0)
        .filter(|(a, b)| in_colour(a) && a != b)
        .count();
    assert_eq!(
        differ, 0,
        "a selected-text colour leaves colour glyphs as they are"
    );
}
