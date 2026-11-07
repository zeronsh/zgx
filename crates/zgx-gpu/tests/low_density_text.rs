//! Text below 2x goes on whole device pixels: a glyph repeated along a
//! line lands on the same pixel phase each time, so it is one image in the
//! atlas. At 2x it keeps its quarter-pixel places, one image per place.
use zgx::scene::*;
use zgx_gpu::{GpuRenderer, text::FontData};

const GEIST: &[u8] = include_bytes!("fonts/GeistVariable-subset.ttf");

/// Glyph images uploaded drawing `text` in Geist at `scale`.
fn uploads(text: &str, scale: f32) -> usize {
    let mut gpu =
        GpuRenderer::new((400. * scale) as u32, (40. * scale) as u32).expect("a GPU adapter");
    gpu.set_scale_factor(scale);
    FontData::new(GEIST.to_vec())
        .unwrap()
        .install(&mut gpu.text_system().borrow_mut());
    let mut scene = Scene::new(400., 40.);
    gpu.install_text(&mut scene);
    let node = scene.append(
        scene.root(),
        NodeKind::Text {
            text: text.into(),
            font_size: 13.,
            color: Color(255, 255, 255, 255),
        },
        Style {
            width: Some(400.),
            height: Some(40.),
            ..Default::default()
        },
    );
    scene.set_font(
        node,
        zgx::text_layout::FontStyle {
            family: "Geist".into(),
            ..Default::default()
        },
    );
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap().glyph_uploads
}

#[test]
fn a_glyph_repeated_along_a_line_is_one_image_below_2x() {
    let line = "l".repeat(24);
    assert_eq!(uploads(&line, 1.), 1, "one place, one image at 1x");
    assert_eq!(uploads(&line, 1.5), 1, "one place, one image at 1.5x");
    assert!(
        uploads(&line, 2.) > 1,
        "at 2x a glyph keeps its quarter-pixel places"
    );
}

/// A centred line whose box grows, so only its start moves, keeps its spacing
/// below 2x: each column of its ink the same, frame after frame, not a letter
/// a pixel nearer its neighbour as the line's start passes each half pixel.
#[test]
fn a_line_that_only_moves_keeps_its_spacing_below_2x() {
    use zgx::text_layout::{FontStyle, TextAlign};
    let scale = 1.;
    let mut gpu = GpuRenderer::new(400, 40).expect("a GPU adapter");
    gpu.set_scale_factor(scale);
    FontData::new(GEIST.to_vec())
        .unwrap()
        .install(&mut gpu.text_system().borrow_mut());
    let mut lines = Vec::new();
    for step in 0..16 {
        let mut scene = Scene::new(400., 40.);
        gpu.install_text(&mut scene);
        let node = scene.append(
            scene.root(),
            NodeKind::Text {
                text: "minimum illicit".into(),
                font_size: 13.,
                color: Color(255, 255, 255, 255),
            },
            Style {
                width: Some(300. + step as f32 * 0.25),
                height: Some(40.),
                ..Default::default()
            },
        );
        scene.set_font(
            node,
            FontStyle {
                family: "Geist".into(),
                align: TextAlign::Center,
                ..Default::default()
            },
        );
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let pixels = gpu.readback().unwrap();
        let columns: Vec<u32> = (0..400)
            .map(|x| (0..40).map(|y| pixels[(y * 400 + x) * 4] as u32).sum())
            .collect();
        let first = columns.iter().position(|ink| *ink > 0).unwrap();
        let last = columns.iter().rposition(|ink| *ink > 0).unwrap();
        lines.push(columns[first..=last].to_vec());
    }
    assert!(lines.iter().all(|line| *line == lines[0]));
}
