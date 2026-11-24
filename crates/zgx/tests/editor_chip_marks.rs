//! A text area's chips are one drawing: its atoms' washes and the spans a
//! highlighter styles as chips (`SpanStyle::chip`) are marks behind the
//! text, under the selection, each with its own corners. Existing atoms
//! draw exactly as they did: their marks' pieces, at 1x and 2x, are the
//! ones main drew before chips joined them.
use std::sync::Arc;
use zgx::{
    compose::prelude::*,
    scene::{Color, NodeId},
    svg::SvgData,
    text_edit::Atom,
    widgets::{AtomIcon, AtomStyle, Ui},
};

const WASH: Color = Color(0, 200, 0, 255);

fn icon() -> AtomIcon {
    AtomIcon {
        svg: Arc::new(
            SvgData::new(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="8"/></svg>"#
                    .to_vec(),
            )
            .unwrap(),
        ),
        size: 12.,
        gap: 3.,
        bg: None,
        radius: 0.,
        pad: 0.,
    }
}

/// The ways atoms are drawn today, by name.
fn looks() -> Vec<(&'static str, Option<AtomStyle>, f32)> {
    vec![
        ("wash", None, 300.),
        (
            "padded",
            Some(AtomStyle {
                padding: Some([6., 4., 2.]),
                radius: Some(5.),
                ..Default::default()
            }),
            300.,
        ),
        (
            "icon",
            Some(AtomStyle {
                icon: Some(icon()),
                padding: Some([3., 3., 1.]),
                radius: Some(8.),
                ..Default::default()
            }),
            300.,
        ),
        // Narrow: the atom wraps onto a second line.
        (
            "wrapped",
            Some(AtomStyle {
                padding: Some([2., 2., 0.]),
                radius: Some(4.),
                ..Default::default()
            }),
            70.,
        ),
    ]
}

/// The text node that carries `ui`'s decoration marks.
fn marked(ui: &Ui) -> Option<NodeId> {
    let s = ui.scene.borrow();
    let mut stack = vec![s.root()];
    while let Some(n) = stack.pop() {
        if !s.text_marks(n).is_empty() {
            return Some(n);
        }
        stack.extend(s.children(n).iter().copied());
    }
    None
}

/// The pieces atom look `name`'s marks are drawn as, at 1x and 2x.
fn pieces(style: Option<AtomStyle>, width: f32) -> String {
    let mut ui = Ui::new(400., 200.);
    let value = ui.signal(String::from("see @file.pdf now"));
    let mut area = text_area("Notes", value)
        .w(width)
        .h(120.)
        .text_size(16.)
        .atoms_when(|editor| vec![Atom::new(editor.new_atom_id(), 4..13)])
        .atom_wash(WASH);
    if let Some(style) = style {
        area = area.atom_style(style);
    }
    let _view = ui.mount(column().size(400., 200.).child(area));
    ui.prepare_frame();
    ui.prepare_frame();
    let node = marked(&ui).expect("the atom's wash, as marks");
    let s = ui.scene.borrow();
    let at = |scale| format!("{:?}", s.text_mark_pieces(node, 0., 0., scale));
    format!("1x {}\n2x {}", at(1.), at(2.))
}

/// The pieces main drew each atom look's marks as, before chips joined
/// them (captured on 7b3641116, at 1x and 2x).
const MAIN: [(&str, &str); 4] = [
    (
        "wash",
        "1x Some([(Run { rect: Rect { x: 38.0, y: 0.0, width: 87.0, height: 23.0 }, radii: [3.0, 3.0, 3.0, 3.0] }, Color(0, 200, 0, 255))])\n2x Some([(Run { rect: Rect { x: 38.5, y: 0.0, width: 86.5, height: 23.0 }, radii: [3.0, 3.0, 3.0, 3.0] }, Color(0, 200, 0, 255))])",
    ),
    (
        "padded",
        "1x Some([(Run { rect: Rect { x: 38.0, y: 0.0, width: 97.0, height: 23.0 }, radii: [5.0, 5.0, 5.0, 5.0] }, Color(0, 200, 0, 255))])\n2x Some([(Run { rect: Rect { x: 38.5, y: 0.0, width: 96.5, height: 23.0 }, radii: [5.0, 5.0, 5.0, 5.0] }, Color(0, 200, 0, 255))])",
    ),
    (
        "icon",
        "1x Some([(Run { rect: Rect { x: 38.0, y: 1.0, width: 108.0, height: 21.0 }, radii: [8.0, 8.0, 8.0, 8.0] }, Color(0, 200, 0, 255))])\n2x Some([(Run { rect: Rect { x: 38.5, y: 1.0, width: 107.5, height: 21.0 }, radii: [8.0, 8.0, 8.0, 8.0] }, Color(0, 200, 0, 255))])",
    ),
    (
        "wrapped",
        "1x Some([(Run { rect: Rect { x: 38.0, y: 2.0, width: 91.0, height: 19.0 }, radii: [4.0, 4.0, 4.0, 4.0] }, Color(0, 200, 0, 255))])\n2x Some([(Run { rect: Rect { x: 38.5, y: 2.0, width: 90.5, height: 19.0 }, radii: [4.0, 4.0, 4.0, 4.0] }, Color(0, 200, 0, 255))])",
    ),
];

#[test]
fn atoms_draw_exactly_as_they_did() {
    let looks = looks();
    for (name, want) in MAIN {
        let (_, style, width) = looks.iter().find(|(n, ..)| *n == name).cloned().unwrap();
        assert_eq!(pieces(style, width), want, "{name}");
    }
}

#[test]
fn a_chip_span_is_a_mark_with_its_own_corners() {
    let mut ui = Ui::new(400., 200.);
    let value = ui.signal(String::from("see @file.pdf now"));
    let area = text_area("Notes", value)
        .w(300.)
        .h(120.)
        .text_size(16.)
        .span_highlighter(|line, state| {
            let spans = line
                .find("@file.pdf")
                .map(|at| {
                    (
                        at..at + 9,
                        zgx::widgets::SpanStyle::default()
                            .background(WASH)
                            .chip([4., 2.], 7.),
                    )
                })
                .into_iter()
                .collect();
            (spans, state)
        });
    let _view = ui.mount(column().size(400., 200.).child(area));
    ui.prepare_frame();
    ui.prepare_frame();
    let node = marked(&ui).expect("the chip, as a mark");
    let s = ui.scene.borrow();
    let pieces = s.text_mark_pieces(node, 0., 0., 1.).unwrap();
    assert_eq!(pieces.len(), 1, "{pieces:?}");
    let (piece, color) = &pieces[0];
    assert_eq!(*color, WASH);
    match piece {
        zgx::scene::HighlightPiece::Run { rect, radii } => {
            assert_eq!(*radii, [7.; 4], "its own corners");
            // Its padding down hugs the text: 16·1.16 + 2·2 = 22.56 points.
            assert!((rect.height - 23.).abs() <= 1., "{rect:?}");
        }
        other => panic!("one run: {other:?}"),
    }
}
