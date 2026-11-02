//! A wrapping text measured with the widths it holds for
//! (`Scene::set_font_text_reach_measurer`) is not measured again at a width
//! between its widest line's and its reach: a window dragged a few points
//! keeps its lines. Past either end it is measured again.
use std::{cell::Cell, rc::Rc};
use zgx::scene::{Color, Layout, NodeId, NodeKind, Scene, Style};

/// A page `width` wide holding one wrapping text, measured by a shaper that
/// says its lines hold from their widest (180) up to 260, and counts calls.
fn scene(width: f32, calls: &Rc<Cell<usize>>) -> (Scene, NodeId, NodeId) {
    let mut scene = Scene::new(640., 480.);
    let counted = calls.clone();
    scene.set_font_text_reach_measurer(move |_, _, limit, _| {
        counted.set(counted.get() + 1);
        match limit {
            Some(limit) if (180. ..260.).contains(&limit) => ((180., 40.), 260.),
            Some(limit) if limit < 180. => ((limit.min(120.), 60.), 180.),
            _ => ((300., 20.), f32::INFINITY),
        }
    });
    let root = scene.root();
    let page = scene.append(
        root,
        NodeKind::Container(Layout::Column),
        Style {
            width: Some(width),
            ..Style::default()
        },
    );
    let text = scene.append(
        page,
        NodeKind::Text {
            text: "a paragraph that wraps".into(),
            color: Color(0, 0, 0, 255),
            font_size: 14.,
        },
        Style {
            text_wrap: true,
            ..Style::default()
        },
    );
    scene.flush();
    (scene, page, text)
}

fn resize(scene: &mut Scene, page: NodeId, width: f32) {
    scene.set_style(
        page,
        Style {
            width: Some(width),
            ..Style::default()
        },
    );
    scene.flush();
}

#[test]
fn a_width_within_a_texts_reach_keeps_its_lines_unmeasured() {
    let calls = Rc::new(Cell::new(0));
    let (mut scene, page, text) = scene(200., &calls);
    assert_eq!(scene.bounds(text).height, 40.);
    let measured = calls.get();
    assert!(measured > 0);
    // Dragged a few points at a time, within 180..260: served as it was.
    for width in [203., 197., 241., 180., 259.] {
        resize(&mut scene, page, width);
        assert_eq!(scene.bounds(text).height, 40., "at {width}");
    }
    assert_eq!(
        calls.get(),
        measured,
        "no width within its reach is measured"
    );
    // Past its reach, and under its widest line: measured again.
    resize(&mut scene, page, 260.);
    assert_eq!(scene.bounds(text).height, 20.);
    assert!(
        calls.get() > measured,
        "its reach is where it is measured again"
    );
    let measured = calls.get();
    resize(&mut scene, page, 179.);
    assert_eq!(scene.bounds(text).height, 60.);
    assert!(
        calls.get() > measured,
        "under its widest line it is measured again"
    );
}
