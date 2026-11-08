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

/// Below 2x a mask's edges firm by how light its text is, as Windows' own text
/// does: white on black gains weight on its strokes' edges, near-black on white
/// keeps its own. At 2x neither changes.
#[test]
fn only_light_text_is_firmed_below_2x() {
    fn ink(scale: f32, ground: Color, text: Color) -> f32 {
        let (width, height) = ((400. * scale) as u32, (40. * scale) as u32);
        let mut gpu = GpuRenderer::new(width, height).expect("a GPU adapter");
        gpu.set_scale_factor(scale);
        FontData::new(GEIST.to_vec())
            .unwrap()
            .install(&mut gpu.text_system().borrow_mut());
        let mut scene = Scene::new(400., 40.);
        gpu.install_text(&mut scene);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let box_ = Style {
            width: Some(400.),
            height: Some(40.),
            ..Default::default()
        };
        scene.append(scene.root(), NodeKind::Rect(ground), box_.clone());
        let node = scene.append(
            scene.root(),
            NodeKind::Text {
                text: "minimum illicit filling".into(),
                font_size: 13.,
                color: text,
            },
            box_,
        );
        scene.set_font(
            node,
            zgx::text_layout::FontStyle {
                family: "Geist".into(),
                ..Default::default()
            },
        );
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let span = text.0 as f32 - ground.0 as f32;
        gpu.readback()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|px| (px[0] as f32 - ground.0 as f32) / span)
            .sum()
    }
    let (black, white) = (Color(0, 0, 0, 255), Color(255, 255, 255, 255));
    let (light, dark) = (ink(1., black, white), ink(1., white, black));
    assert!(light > dark * 1.03, "{light} on black, {dark} on white");
    let (light, dark) = (ink(2., black, white), ink(2., white, black));
    assert!(
        (light - dark).abs() < dark * 0.002,
        "{light} and {dark} at 2x"
    );
}

/// Below 2x the firming stays on a stroke's own edges: the faint column a stem
/// spills into its neighbour reads the same, white on black, as near-black on
/// white, which isn't firmed. Lifted, a one-pixel stem reads as a soft
/// two-pixel one.
#[test]
fn a_stems_faint_neighbour_is_not_firmed_below_2x() {
    // Each column's strongest coverage across a line of `l`s at `size`, 0 to 1.
    fn columns(size: f32, ground: Color, text: Color) -> Vec<f32> {
        let mut gpu = GpuRenderer::new(400, 40).expect("a GPU adapter");
        gpu.set_scale_factor(1.);
        FontData::new(GEIST.to_vec())
            .unwrap()
            .install(&mut gpu.text_system().borrow_mut());
        let mut scene = Scene::new(400., 40.);
        gpu.install_text(&mut scene);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let box_ = Style {
            width: Some(400.),
            height: Some(40.),
            ..Default::default()
        };
        scene.append(scene.root(), NodeKind::Rect(ground), box_.clone());
        let node = scene.append(
            scene.root(),
            NodeKind::Text {
                text: "l".repeat(12).into(),
                font_size: size,
                color: text,
            },
            box_,
        );
        scene.set_font(
            node,
            zgx::text_layout::FontStyle {
                family: "Geist".into(),
                ..Default::default()
            },
        );
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let pixels = gpu.readback().unwrap();
        let span = text.0 as f32 - ground.0 as f32;
        (0..400)
            .map(|x| {
                (0..40)
                    .map(|y| (pixels[(y * 400 + x) * 4] as f32 - ground.0 as f32) / span)
                    .fold(0., f32::max)
            })
            .collect()
    }
    let (black, white) = (Color(0, 0, 0, 255), Color(255, 255, 255, 255));
    // A stem's spill depends on where it falls in its pixel: across a few
    // sizes, some are faint.
    let faint: Vec<(f32, f32)> = (12..=18)
        .flat_map(|size| {
            let size = size as f32;
            let (light, dark) = (columns(size, black, white), columns(size, white, black));
            dark.into_iter()
                .zip(light)
                .filter(|(dark, _)| *dark > 0.05 && *dark < 0.3)
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(!faint.is_empty(), "no stem spilled a faint column");
    for (dark, light) in faint {
        assert!(
            (light - dark).abs() < 0.03,
            "a faint column {light} white on black, {dark} on white"
        );
    }
}
