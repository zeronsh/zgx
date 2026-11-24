//! `text::set_font_smoothing` changes how heavy macOS text is drawn, on a
//! renderer that has drawn it already: off, white text on black draws
//! lighter, as Core Text's outlines alone; back to the system's, as heavy as
//! before. A binary of its own: the setting is the process's.
#![cfg(target_os = "macos")]
use zgx::scene::*;
use zgx_gpu::{GpuRenderer, text, text::FontData};

const GEIST: &[u8] = include_bytes!("fonts/GeistVariable-subset.ttf");

#[test]
fn turning_smoothing_off_draws_text_lighter_until_turned_back() {
    let (width, height) = (400., 40.);
    let mut gpu = GpuRenderer::new(800, 80).expect("a GPU adapter");
    gpu.set_scale_factor(2.);
    FontData::new(GEIST.to_vec())
        .unwrap()
        .install(&mut gpu.text_system().borrow_mut());
    let mut scene = Scene::new(width, height);
    gpu.install_text(&mut scene);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let whole = Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    };
    scene.append(
        scene.root(),
        NodeKind::Rect(Color(0, 0, 0, 255)),
        whole.clone(),
    );
    let node = scene.append(
        scene.root(),
        NodeKind::Text {
            text: "minimum illicit filling".into(),
            font_size: 13.,
            color: Color(255, 255, 255, 255),
        },
        whole,
    );
    scene.set_font(
        node,
        zgx::text_layout::FontStyle {
            family: "Geist".into(),
            ..Default::default()
        },
    );
    let damage = scene.flush().damage;
    let mut ink = |damage: &[Rect]| -> f32 {
        gpu.render(&scene, damage).unwrap();
        gpu.readback()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|px| f32::from(px[0]) / 255.)
            .sum()
    };
    let system = ink(&damage);
    // Nothing changed in the scene: the window's redraw, all of it.
    let all = [Rect {
        x: 0.,
        y: 0.,
        width,
        height,
    }];
    text::set_font_smoothing(Some(false));
    let off = ink(&all);
    text::set_font_smoothing(None);
    let again = ink(&all);
    if text::smoothing(Color(255, 255, 255, 255)) == 0 {
        // The user turned macOS's smoothing off: light either way.
        assert!((system - off).abs() < system * 0.002, "{system} and {off}");
        return;
    }
    assert!(off < system * 0.9, "{off} off, {system} as the system's");
    assert!(
        (again - system).abs() < system * 0.002,
        "{again} again, {system} before"
    );
}
