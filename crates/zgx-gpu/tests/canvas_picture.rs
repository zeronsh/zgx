//! `canvas::picture`: a canvas drawn with no window is what the GPU shows
//! of the same canvas in a window, its text included.
use zgx::canvas::{
    Canvas, CanvasText, GradientStop, Path, Point, Stroke, TextAnchor, TextBaseline,
};
use zgx::compose::prelude::*;
use zgx::scene::{CanvasView, Color, Rect};
use zgx::widgets::Ui;
use zgx_gpu::GpuRenderer;

const W: f32 = 240.;
const H: f32 = 160.;

/// Every kind of mark a design draws: fills, a rounded box, a gradient, a
/// curve's stroke, a clip, dashes, and text in each alignment.
fn drawing() -> Canvas {
    let mut c = Canvas::new();
    c.fill(
        Path::rounded_rect(Rect::new(0., 0., W, H), 0.).unwrap(),
        Color(250, 250, 247, 255),
    );
    c.fill(
        Path::rounded_rect(Rect::new(12., 12., 90., 50.), 10.).unwrap(),
        Color(220, 40, 60, 255),
    );
    c.fill(
        Path::rounded_rect(Rect::new(120., 12., 100., 50.), 0.).unwrap(),
        zgx::canvas::Brush::linear(
            Point::new(120., 0.),
            Point::new(220., 0.),
            vec![
                GradientStop {
                    offset: 0.,
                    color: Color(30, 90, 220, 255),
                },
                GradientStop {
                    offset: 1.,
                    color: Color(30, 200, 120, 255),
                },
            ],
        )
        .unwrap(),
    );
    let mut curve = Path::builder();
    curve.move_to(14., 120.);
    curve.cubic_to(
        Point::new(60., 70.),
        Point::new(100., 150.),
        Point::new(150., 90.),
    );
    c.stroke(
        curve.build().unwrap(),
        Color(40, 40, 50, 255),
        Stroke {
            width: 3.,
            ..Default::default()
        },
    );
    c.clip(
        Path::ellipse(Rect::new(160., 80., 60., 60.)).unwrap(),
        zgx::canvas::FillRule::Winding,
    );
    c.fill(
        Path::rounded_rect(Rect::new(150., 70., 80., 40.), 0.).unwrap(),
        Color(250, 180, 30, 200),
    );
    c.restore();
    c.stroke(
        Path::line(Point::new(12., 150.), Point::new(228., 150.)).unwrap(),
        Color(120, 120, 130, 255),
        Stroke {
            width: 1.,
            dash: vec![4., 3.],
            ..Default::default()
        },
    );
    for (text, x, anchor, baseline) in [
        ("Welcome", 14., TextAnchor::Start, TextBaseline::Top),
        ("Sign in", 120., TextAnchor::Center, TextBaseline::Middle),
        ("12:30", 226., TextAnchor::End, TextBaseline::Bottom),
    ] {
        c.text(CanvasText {
            text: text.into(),
            x,
            y: 132.,
            size: 15.,
            color: Color(20, 20, 30, 255),
            font: Default::default(),
            anchor,
            baseline,
        });
    }
    c
}

/// The canvas as a window shows it, read back from the GPU, `scale` pixels
/// a point.
fn on_screen(scale: f32) -> Vec<u8> {
    let (w, h) = ((W * scale) as u32, (H * scale) as u32);
    let mut gpu = GpuRenderer::new(w, h).expect("a GPU adapter");
    gpu.set_scale_factor(scale);
    let mut ui = Ui::new(W, H);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let handle = ui.mount(div().w(W).h(H).child(canvas(|_| drawing()).w(W).h(H)));
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    gpu.render(&ui.scene.borrow(), &damage).unwrap();
    std::mem::forget(handle);
    gpu.readback().unwrap()
}

/// How far two pictures' pixels are apart: the mean of each channel's
/// difference, and the share of pixels any channel of which is more than
/// `far` apart.
fn apart(a: &[u8], b: &[u8], far: u8) -> (f64, f64) {
    assert_eq!(a.len(), b.len());
    let mut sum = 0_u64;
    let mut outliers = 0;
    for (p, q) in a.chunks(4).zip(b.chunks(4)) {
        let mut worst = 0;
        for i in 0..4 {
            let d = p[i].abs_diff(q[i]);
            sum += u64::from(d);
            worst = worst.max(d);
        }
        if worst > far {
            outliers += 1;
        }
    }
    (
        sum as f64 / a.len() as f64,
        outliers as f64 / (a.len() / 4) as f64,
    )
}

#[test]
fn a_picture_is_what_the_gpu_shows_of_the_canvas() {
    let mut text = zgx_gpu::text::CpuTextRaster::default();
    for scale in [1., 2.] {
        let shown = on_screen(scale);
        let picture = zgx_gpu::canvas::picture(
            &drawing(),
            CanvasView::default(),
            W,
            H,
            scale,
            None,
            &mut text,
        )
        .unwrap();
        assert_eq!(
            (picture.width(), picture.height()),
            ((W * scale) as u32, (H * scale) as u32)
        );
        // Measured on Direct3D 12: 0.004 at 1x, 0.001 at 2x, none over 32.
        // The bounds leave room for other GPUs' rounding.
        let (mean, outliers) = apart(picture.pixels(), &shown, 32);
        assert!(mean < 0.25, "scale {scale}: mean channel difference {mean}");
        assert!(
            outliers < 0.001,
            "scale {scale}: {outliers} of pixels differ by over 32"
        );
    }
}

#[test]
fn text_is_where_and_as_wide_as_the_canvas_puts_it() {
    let mut text = zgx_gpu::text::CpuTextRaster::default();
    let mut c = Canvas::new();
    c.text(CanvasText {
        text: "MMMM".into(),
        x: 100.,
        y: 40.,
        size: 20.,
        color: Color(0, 0, 0, 255),
        font: Default::default(),
        anchor: TextAnchor::Center,
        baseline: TextBaseline::Middle,
    });
    let (width, _) = text.measure("MMMM", 20., &Default::default());
    let picture =
        zgx_gpu::canvas::picture(&c, CanvasView::default(), 200., 80., 1., None, &mut text)
            .unwrap();
    // Inked columns: centred on 100, about as wide as measured.
    let inked: Vec<u32> = (0..200)
        .filter(|x| (0..80).any(|y| picture.pixels()[((y * 200 + x) * 4 + 3) as usize] > 128))
        .collect();
    let (left, right) = (inked[0] as f32, *inked.last().unwrap() as f32 + 1.);
    assert!(((left + right) / 2. - 100.).abs() <= 1.5, "{left}..{right}");
    assert!(
        (right - left - width).abs() <= 4.,
        "{left}..{right} for {width}"
    );
}

#[test]
fn a_view_moves_the_drawing_and_its_text_and_a_background_fills_under() {
    let mut text = zgx_gpu::text::CpuTextRaster::default();
    let mut c = Canvas::new();
    c.fill(
        Path::rounded_rect(Rect::new(500., 300., 10., 10.), 0.).unwrap(),
        Color(255, 0, 0, 255),
    );
    let view = CanvasView {
        x: -500.,
        y: -300.,
        scale: 1.,
    };
    let picture = zgx_gpu::canvas::picture(
        &c,
        view,
        20.,
        20.,
        2.,
        Some(Color(0, 0, 255, 255)),
        &mut text,
    )
    .unwrap();
    let at = |x: u32, y: u32| &picture.pixels()[((y * 40 + x) * 4) as usize..][..4];
    assert_eq!(at(5, 5), &[255, 0, 0, 255]);
    assert_eq!(at(30, 30), &[0, 0, 255, 255]);
    let png = zgx_gpu::canvas::png(&picture).unwrap();
    let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .unwrap()
        .to_rgba8();
    assert_eq!(decoded.dimensions(), (40, 40));
    assert_eq!(decoded.as_raw().as_slice(), picture.pixels());
}
