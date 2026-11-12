//! A canvas's text drawn under a transform turns with it on screen, as a
//! shape drawn under the same transform does.
use zgx::affine::Affine;
use zgx::canvas::{Canvas, CanvasText, Path, TextAnchor, TextBaseline};
use zgx::compose::prelude::*;
use zgx::scene::{CanvasView, Color, Rect};
use zgx::widgets::Ui;
use zgx_gpu::GpuRenderer;

const W: f32 = 260.;
const H: f32 = 260.;

fn label() -> CanvasText {
    CanvasText {
        text: "WWWWW".into(),
        x: 120.,
        y: 20.,
        size: 22.,
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

/// `transform` turning the label, shown on the GPU under `view`: its pixels.
fn shown(transform: Affine, view: Option<CanvasView>) -> Vec<u8> {
    let mut gpu = GpuRenderer::new(W as u32, H as u32).unwrap();
    let mut ui = Ui::new(W, H);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let mut board = canvas(move |_| {
        let mut c = Canvas::new();
        c.fill(
            Path::rectangle(Rect::new(0., 0., W, H)),
            Color(255, 255, 255, 255),
        );
        c.text_transformed(label(), transform);
        c
    })
    .w(W)
    .h(H);
    if let Some(view) = view {
        board = board.canvas_view(move || view);
    }
    let handle = ui.mount(div().w(W).h(H).child(board));
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    gpu.render(&ui.scene.borrow(), &damage).unwrap();
    std::mem::forget(handle);
    gpu.readback().unwrap()
}

/// The bounds of the dark pixels: left, top, right, bottom.
fn inked(pixels: &[u8]) -> (usize, usize, usize, usize) {
    let (mut l, mut t, mut r, mut b) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..H as usize {
        for x in 0..W as usize {
            let at = 4 * (y * W as usize + x);
            if pixels[at..at + 3].iter().all(|c| *c < 90) {
                (l, t, r, b) = (l.min(x), t.min(y), r.max(x), b.max(y));
            }
        }
    }
    (l, t, r, b)
}

#[test]
fn a_text_turns_mirrors_and_scales_with_its_transform() {
    let (l, t, r, b) = inked(&shown(Affine::IDENTITY, None));
    let (wide, tall) = (r - l, b - t);
    assert!(wide > 2 * tall, "a line across: {:?}", (l, t, r, b));
    assert!(l >= 120 && t >= 20, "right of and below its point");
    // A quarter turn clockwise about its point (120, 20): it runs down from
    // there, its glyphs' tops facing right, so it lies left of x = 120.
    let quarter = Affine::rotation(std::f32::consts::FRAC_PI_2).around(120., 20.);
    let (ql, qt, qr, qb) = inked(&shown(quarter, None));
    assert!(
        (qb - qt).abs_diff(wide) <= 3 && (qr - ql).abs_diff(tall) <= 3,
        "turned: {:?} from {:?}",
        (ql, qt, qr, qb),
        (l, t, r, b)
    );
    assert!(
        qr <= 120 && qt >= 20,
        "down and left of its point: {:?}",
        (ql, qt, qr, qb)
    );
    // Mirrored across x = 120: the same line, left of its point.
    let mirrored = Affine::scale(-1., 1.).around(120., 20.);
    let (ml, mt, mr, mb) = inked(&shown(mirrored, None));
    assert!(
        mr <= 120 && (mr - ml).abs_diff(wide) <= 2 && (mt, mb) == (t, b),
        "{:?}",
        (ml, mt, mr, mb)
    );
    // Half as wide, twice as tall: what a frame scaled unevenly does.
    let squashed = Affine::scale(0.5, 2.).around(120., 20.);
    let (sl, st, sr, sb) = inked(&shown(squashed, None));
    assert!(
        (sr - sl).abs_diff(wide / 2) <= 3 && (sb - st).abs_diff(tall * 2) <= 4,
        "{:?}",
        (sl, st, sr, sb)
    );
    // Sheared: each row of pixels moved across by half how far below the
    // point it is, the line as tall.
    let sheared = Affine {
        c: 0.5,
        ..Affine::IDENTITY
    }
    .around(120., 20.);
    let (hl, ht, hr, hb) = inked(&shown(sheared, None));
    assert!(
        (ht, hb) == (t, b)
            && hl.abs_diff(l + (t - 20) / 2) <= 2
            && (r + (t - 20) / 2..=r + (b - 20) / 2 + 1).contains(&hr),
        "{:?} from {:?}",
        (hl, ht, hr, hb),
        (l, t, r, b)
    );
    // Turned, then zoomed twice by the canvas's view: both apply.
    let zoomed = inked(&shown(
        quarter,
        Some(CanvasView {
            x: -80.,
            y: -20.,
            scale: 2.,
        }),
    ));
    assert!(
        (zoomed.3 - zoomed.1).abs_diff(2 * wide) <= 6 && zoomed.2 <= 160,
        "{zoomed:?}"
    );
}
