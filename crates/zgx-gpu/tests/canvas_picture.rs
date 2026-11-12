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
            width: None,
        });
    }
    // The app's own font (registered as `Application::font` does), and a
    // text of two lines.
    for (text, x, anchor, font) in [
        ("Stillwater 9:41", 14., TextAnchor::Start, geist_semibold()),
        ("Two\nlines", 226., TextAnchor::End, Default::default()),
    ] {
        c.text(CanvasText {
            text: text.into(),
            x,
            y: 66.,
            size: 13.,
            color: Color(20, 60, 40, 255),
            font,
            anchor,
            baseline: TextBaseline::Top,
            width: None,
        });
    }
    // A paragraph wrapped to 90 points, centred on (120, 37), its lines
    // centred in its box (as `c:text` sets `align` from `anchor`).
    c.text(CanvasText {
        text: "Find your still point, a few minutes a day".into(),
        x: 120.,
        y: 37.,
        size: 11.,
        color: Color(10, 10, 10, 255),
        font: zgx::text_layout::FontStyle {
            align: zgx::text_layout::TextAlign::Center,
            ..Default::default()
        },
        anchor: TextAnchor::Center,
        baseline: TextBaseline::Middle,
        width: Some(90.),
    });
    c
}

fn geist() -> zgx_gpu::text::FontData {
    zgx_gpu::text::FontData::new(
        &include_bytes!("../../zgx-desktop/examples/support/fonts/GeistVariable.ttf")[..],
    )
    .unwrap()
}
fn geist_semibold() -> zgx::text_layout::FontStyle {
    zgx::text_layout::FontStyle {
        family: zgx::text_layout::FontFamily::Named("Geist".into()),
        weight: 600,
        ..Default::default()
    }
}

/// The canvas as a window shows it, read back from the GPU, `scale` pixels
/// a point.
fn on_screen(scale: f32) -> Vec<u8> {
    let (w, h) = ((W * scale) as u32, (H * scale) as u32);
    let mut gpu = GpuRenderer::new(w, h).expect("a GPU adapter");
    gpu.set_scale_factor(scale);
    geist().install(&mut gpu.text_system().borrow_mut());
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
    let fallback = zgx_gpu::text::CpuTextRaster::default().measure(
        "Stillwater 9:41",
        13.,
        None,
        &geist_semibold(),
    );
    let mut text = zgx_gpu::text::CpuTextRaster::default();
    text.register_font(&geist());
    // Geist, not the face it falls back to without it.
    let geist = text.measure("Stillwater 9:41", 13., None, &geist_semibold());
    assert_ne!(geist.0, fallback.0);
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
    // Geist, so the face (and its side bearings) is the same on every
    // system, not each one's fallback.
    let mut text = zgx_gpu::text::CpuTextRaster::default();
    text.register_font(&geist());
    let mut c = Canvas::new();
    c.text(CanvasText {
        text: "MMMM".into(),
        x: 100.,
        y: 40.,
        size: 20.,
        color: Color(0, 0, 0, 255),
        font: geist_semibold(),
        anchor: TextAnchor::Center,
        baseline: TextBaseline::Middle,
        width: None,
    });
    let (width, _) = text.measure("MMMM", 20., None, &geist_semibold());
    let picture =
        zgx_gpu::canvas::picture(&c, CanvasView::default(), 200., 80., 1., None, &mut text)
            .unwrap();
    // Inked columns: centred on 100, as wide as measured less the first
    // and last M's side bearings (Geist 600: 66..133 for 71.2).
    let inked: Vec<u32> = (0..200)
        .filter(|x| (0..80).any(|y| picture.pixels()[((y * 200 + x) * 4 + 3) as usize] > 128))
        .collect();
    let (left, right) = (inked[0] as f32, *inked.last().unwrap() as f32 + 1.);
    assert!(((left + right) / 2. - 100.).abs() <= 1.5, "{left}..{right}");
    assert!(
        right - left <= width + 1. && right - left >= width - 6.,
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

#[test]
fn a_centred_paragraphs_short_lines_are_centred_on_its_point() {
    let mut text = zgx_gpu::text::CpuTextRaster::default();
    for baseline in [TextBaseline::Top, TextBaseline::Middle] {
        let mut c = Canvas::new();
        c.fill(
            zgx::canvas::Path::rectangle(Rect::new(0., 0., 200., 100.)),
            Color(255, 255, 255, 255),
        );
        c.text(CanvasText {
            text: "MM\nMM".into(),
            x: 100.,
            y: 40.,
            size: 16.,
            color: Color(0, 0, 0, 255),
            font: zgx::text_layout::FontStyle {
                align: zgx::text_layout::TextAlign::Center,
                ..Default::default()
            },
            anchor: TextAnchor::Center,
            baseline,
            width: None,
        });
        let picture =
            zgx_gpu::canvas::picture(&c, CanvasView::default(), 200., 100., 1., None, &mut text)
                .unwrap();
        let inked: Vec<u32> = (0..200)
            .filter(|x| (0..100).any(|y| picture.pixels()[((y * 200 + x) * 4) as usize] < 128))
            .collect();
        let (left, right) = (inked[0] as f32, *inked.last().unwrap() as f32 + 1.);
        assert!(
            ((left + right) / 2. - 100.).abs() <= 1.5,
            "{baseline:?}: {left}..{right}"
        );
        // And on screen.
        let mut gpu = GpuRenderer::new(200, 100).expect("a GPU adapter");
        let mut ui = Ui::new(200., 100.);
        gpu.install_text(&mut ui.scene.borrow_mut());
        let drawn = c.clone();
        let handle = ui.mount(
            div()
                .w(200.)
                .h(100.)
                .child(canvas(move |_| drawn.clone()).w(200.).h(100.)),
        );
        ui.prepare_frame();
        let damage = ui.scene.borrow_mut().flush().damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        std::mem::forget(handle);
        let shown = gpu.readback().unwrap();
        let inked: Vec<u32> = (0..200)
            .filter(|x| (0..100).any(|y| shown[((y * 200 + x) * 4) as usize] < 128))
            .collect();
        let (left, right) = (inked[0] as f32, *inked.last().unwrap() as f32 + 1.);
        assert!(
            ((left + right) / 2. - 100.).abs() <= 1.5,
            "on screen, {baseline:?}: {left}..{right}"
        );
    }
}

/// `c` as a window shows it, `w` by `h` points at `scale`, read back.
fn shown_of(c: &Canvas, (w, h): (f32, f32), scale: f32) -> Vec<u8> {
    let mut gpu = GpuRenderer::new((w * scale) as u32, (h * scale) as u32).expect("a GPU adapter");
    gpu.set_scale_factor(scale);
    let mut ui = Ui::new(w, h);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let drawn = c.clone();
    let handle = ui.mount(
        div()
            .w(w)
            .h(h)
            .child(canvas(move |_| drawn.clone()).w(w).h(h)),
    );
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    gpu.render(&ui.scene.borrow(), &damage).unwrap();
    std::mem::forget(handle);
    gpu.readback().unwrap()
}

/// Text under a shape drawn after it, text cut by a clip, and text turned:
/// a picture shows each as a window does.
#[test]
fn layers_clips_turned_text_and_gradients_render_as_on_screen() {
    use zgx::affine::Affine;
    let (w, h) = (240., 160.);
    let label = |text: &str, x: f32, y: f32| CanvasText {
        text: text.into(),
        x,
        y,
        size: 18.,
        color: Color(10, 10, 20, 255),
        font: zgx::text_layout::FontStyle {
            weight: 700,
            ..Default::default()
        },
        anchor: TextAnchor::Start,
        baseline: TextBaseline::Top,
        width: None,
    };
    let stops = vec![
        GradientStop {
            offset: 0.,
            color: Color(250, 200, 40, 255),
        },
        GradientStop {
            offset: 1.,
            color: Color(40, 90, 220, 255),
        },
    ];
    let mut c = Canvas::new();
    c.fill(
        Path::rounded_rect(Rect::new(0., 0., w, h), 0.).unwrap(),
        Color(255, 255, 255, 255),
    );
    // A card's title, then a second card over its left half.
    c.text(label("Overlapped", 8., 8.));
    c.fill(
        Path::rounded_rect(Rect::new(0., 0., 60., 40.), 6.).unwrap(),
        zgx::canvas::Brush::linear(
            zgx::canvas::Point::new(0., 0.),
            zgx::canvas::Point::new(60., 40.),
            stops.clone(),
        )
        .unwrap(),
    );
    c.text(label("On top", 8., 44.));
    // A frame clipping its text to a rounded box.
    c.clip(
        Path::rounded_rect(Rect::new(120., 10., 100., 50.), 12.).unwrap(),
        zgx::canvas::FillRule::Winding,
    );
    c.text(label("Clipped text runs on", 100., 20.));
    c.restore();
    // A dial with a turned label beside it.
    c.fill(
        Path::ellipse(Rect::new(20., 80., 70., 70.)).unwrap(),
        zgx::canvas::Brush::linear(
            zgx::canvas::Point::new(20., 80.),
            zgx::canvas::Point::new(90., 150.),
            stops,
        )
        .unwrap(),
    );
    c.text_transformed(
        label("Turned", 150., 80.),
        Affine::rotation(0.5).around(150., 80.),
    );
    let mut text = zgx_gpu::text::CpuTextRaster::default();
    for scale in [1., 2.] {
        let shown = shown_of(&c, (w, h), scale);
        let picture =
            zgx_gpu::canvas::picture(&c, CanvasView::default(), w, h, scale, None, &mut text)
                .unwrap();
        // Measured on Direct3D 12: 0.007 at 1x, 0.002 at 2x, none over
        // 32 (nightly's picture before: 4.7, and 3.5% over 48).
        let (mean, outliers) = apart(picture.pixels(), &shown, 32);
        assert!(mean < 0.25, "scale {scale}: mean channel difference {mean}");
        assert!(
            outliers < 0.001,
            "scale {scale}: {outliers} of pixels differ by over 32"
        );
    }
}
