//! A canvas's text on screen: in its drawing's order, under a shape drawn
//! after it and over one drawn before, and cut by the clips open where it
//! was drawn.
use zgx::canvas::{Canvas, CanvasText, FillRule, Path, TextAnchor, TextBaseline};
use zgx::compose::prelude::*;
use zgx::scene::{CanvasView, Color, NodeKind, Rect};
use zgx::widgets::Ui;
use zgx_gpu::GpuRenderer;

const W: f32 = 160.;
const H: f32 = 100.;
const WHITE: Color = Color(255, 255, 255, 255);
const RED: Color = Color(220, 30, 30, 255);

fn label(text: &str, x: f32, y: f32, size: f32) -> CanvasText {
    CanvasText {
        text: text.into(),
        x,
        y,
        size,
        color: Color(0, 0, 0, 255),
        font: zgx::text_layout::FontStyle {
            weight: 900,
            ..Default::default()
        },
        anchor: TextAnchor::Start,
        baseline: TextBaseline::Top,
        width: None,
    }
}

/// `draw` shown on the GPU in a `W` by `H` window, under `view`: its pixels,
/// and how many canvas nodes the scene holds.
fn shown(draw: impl Fn() -> Canvas + 'static, view: Option<CanvasView>) -> (Vec<u8>, usize) {
    let mut gpu = GpuRenderer::new(W as u32, H as u32).unwrap();
    let mut ui = Ui::new(W, H);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let mut board = canvas(move |_| draw()).w(W).h(H);
    if let Some(view) = view {
        board = board.canvas_view(move || view);
    }
    let handle = ui.mount(div().w(W).h(H).child(board));
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    gpu.render(&ui.scene.borrow(), &damage).unwrap();
    let canvases = {
        let scene = ui.scene.borrow();
        let mut count = 0;
        let mut stack = vec![scene.root()];
        while let Some(node) = stack.pop() {
            count += usize::from(matches!(scene.kind(node), NodeKind::Canvas(_)));
            stack.extend(scene.children(node).iter().copied());
        }
        count
    };
    std::mem::forget(handle);
    (gpu.readback().unwrap(), canvases)
}

/// How many pixels within `area` are dark (ink), and how many are neither
/// ink nor `ground` within 40.
fn ink(pixels: &[u8], area: Rect, ground: Color) -> (usize, usize) {
    let (mut dark, mut other) = (0, 0);
    for y in area.y as usize..(area.y + area.height) as usize {
        for x in area.x as usize..(area.x + area.width) as usize {
            let at = 4 * (y * W as usize + x);
            let p = &pixels[at..at + 3];
            if p.iter().all(|c| *c < 90) {
                dark += 1;
            }
            if p.iter()
                .zip([ground.0, ground.1, ground.2])
                .any(|(a, b)| a.abs_diff(b) > 40)
            {
                other += 1;
            }
        }
    }
    (dark, other)
}

#[test]
fn a_shape_drawn_after_a_text_covers_it_and_one_before_lies_under_it() {
    let draw = || {
        let mut c = Canvas::new();
        c.fill(Path::rectangle(Rect::new(0., 0., W, H)), WHITE);
        c.text(label("WWWWWW", 4., 4., 28.));
        // Over the first text's left half.
        c.fill(Path::rectangle(Rect::new(0., 0., 60., H)), RED);
        // A text over it.
        c.text(label("WW", 4., 55., 28.));
        c
    };
    for view in [
        None,
        Some(CanvasView {
            x: 0.,
            y: 0.,
            scale: 1.,
        }),
    ] {
        let (pixels, canvases) = shown(draw, view);
        assert_eq!(canvases, 2, "a layer for what covers the text");
        let (_, other) = ink(&pixels, Rect::new(0., 0., 56., 40.), RED);
        assert_eq!(other, 0, "the first text hidden under the red ({view:?})");
        let (dark, _) = ink(&pixels, Rect::new(64., 4., 90., 34.), WHITE);
        assert!(dark > 50, "its right half shows: {dark}");
        let (dark, _) = ink(&pixels, Rect::new(4., 55., 52., 34.), RED);
        assert!(dark > 50, "the second text over the red: {dark}");
    }
    // Zoomed twice and moved: the layers move together.
    let (pixels, _) = shown(
        draw,
        Some(CanvasView {
            x: -40.,
            y: 0.,
            scale: 2.,
        }),
    );
    // The red spans 0 to 80 across now; the first text runs past it.
    let (_, other) = ink(&pixels, Rect::new(0., 0., 76., 70.), RED);
    assert_eq!(other, 0, "hidden, zoomed");
    let (dark, _) = ink(&pixels, Rect::new(84., 8., 70., 60.), WHITE);
    assert!(dark > 100, "shown past the red, zoomed: {dark}");
}

#[test]
fn a_text_clear_of_what_follows_draws_in_one_layer() {
    let (pixels, canvases) = shown(
        || {
            let mut c = Canvas::new();
            c.fill(Path::rectangle(Rect::new(0., 0., W, H)), WHITE);
            c.text(label("WW", 4., 4., 28.));
            c.fill(Path::rectangle(Rect::new(100., 60., 40., 30.)), RED);
            c
        },
        None,
    );
    assert_eq!(canvases, 1);
    let (dark, _) = ink(&pixels, Rect::new(4., 4., 60., 34.), WHITE);
    assert!(dark > 50, "{dark}");
}

#[test]
fn a_clip_cuts_the_text_drawn_while_it_is_open() {
    let draw = |round: bool| {
        move || {
            let mut c = Canvas::new();
            c.fill(Path::rectangle(Rect::new(0., 0., W, H)), WHITE);
            let card = Rect::new(20., 10., 80., 80.);
            let path = if round {
                Path::ellipse(card).unwrap()
            } else {
                Path::rectangle(card)
            };
            c.clip(path, FillRule::Winding);
            // Lines running past the card on both sides.
            for row in 0..5 {
                c.text(label("WWWWWWWW", 0., row as f32 * 20., 22.));
            }
            c.restore();
            // Not cut: drawn after the clip closed.
            c.text(label("WW", 108., 60., 22.));
            c
        }
    };
    let (pixels, _) = shown(draw(false), None);
    let (_, other) = ink(&pixels, Rect::new(0., 0., 19., H), WHITE);
    assert_eq!(other, 0, "nothing left of the card");
    let (_, other) = ink(&pixels, Rect::new(101., 0., 6., H), WHITE);
    assert_eq!(other, 0, "nothing right of it");
    let (dark, _) = ink(&pixels, Rect::new(20., 10., 80., 80.), WHITE);
    assert!(dark > 400, "inside it: {dark}");
    // Its top left 10 by 10, all of it past a circle of radius 40.
    let corner = Rect::new(20., 10., 10., 10.);
    let (square_corner, _) = ink(&pixels, corner, WHITE);
    assert!(square_corner > 5, "ink in its corner: {square_corner}");
    let (dark, _) = ink(&pixels, Rect::new(108., 60., 50., 30.), WHITE);
    assert!(dark > 50, "after the clip: {dark}");
    // A circle cuts the corners off.
    let (pixels, _) = shown(draw(true), None);
    let (_, other) = ink(&pixels, corner, WHITE);
    assert_eq!(other, 0, "outside the circle");
    let (dark, _) = ink(&pixels, Rect::new(40., 30., 40., 40.), WHITE);
    assert!(dark > 200, "inside it: {dark}");
}
