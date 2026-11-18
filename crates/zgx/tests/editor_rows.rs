//! A text area's lines laid side by side (`LineInset::joins_next`): a table's cells, each a line, drawn in their
//! cells with their marks as tall as the row.
use std::rc::Rc;
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers},
    scene::{Color, NodeId, NodeKind, Rect},
    widgets::{EditorHandle, LineInset, LineMark, Ui},
};

const TEXT: &str = "before\nA1 with text long enough to wrap in its cell\nB1\nA2\nB2\nafter";
const BORDER: Color = Color(200, 0, 0, 255);

fn cell(i: usize) -> Option<(f32, bool)> {
    match i {
        1 | 3 => Some((0., true)),
        2 | 4 => Some((110., false)),
        _ => None,
    }
}
fn table() -> (Ui, EditorHandle) {
    let mut ui = Ui::new(500., 400.);
    let value = ui.signal(TEXT.to_string());
    let view = ui.mount(
        text_area("Notes", value)
            .w(400.)
            .h(300.)
            .p(0.)
            .text_size(10.)
            .line_height(20.)
            .text_wrap(true),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_line_insets(Some(Rc::new(|i| match cell(i) {
        Some((left, joins_next)) => LineInset {
            left,
            width: Some(100.),
            joins_next,
            ..LineInset::default()
        },
        None => LineInset::default(),
    })));
    editor.set_line_marks(Some(Rc::new(|i| match cell(i) {
        Some((left, _)) => vec![LineMark {
            x: left,
            width: f32::INFINITY,
            height: f32::INFINITY,
            border_color: BORDER,
            border_width: 1.,
            ..LineMark::default()
        }],
        None => Vec::new(),
    })));
    ui.input.focus(&ui.scene, Some(view.node()));
    ui.prepare_frame();
    (ui, editor)
}
/// Every node under `root` that `pick` takes, with its place.
fn nodes<T>(ui: &Ui, root: NodeId, pick: impl Fn(&NodeKind) -> Option<T>) -> Vec<(T, Rect)> {
    let scene = ui.scene.borrow();
    let mut found = Vec::new();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        if let Some(value) = pick(scene.kind(id)) {
            found.push((value, scene.bounds(id)));
        }
        pending.extend(scene.children(id).iter().copied());
    }
    found
}
fn line(ui: &Ui, editor: &EditorHandle, text: &str) -> Rect {
    let lines = nodes(ui, editor.node, |kind| match kind {
        NodeKind::Text { text, .. } => Some(text.to_string()),
        _ => None,
    });
    lines
        .into_iter()
        .find(|(shown, _)| shown.starts_with(text))
        .unwrap_or_else(|| panic!("a node for {text}"))
        .1
}
fn press(ui: &mut Ui, key: Key) {
    ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    ui.prepare_frame();
}

#[test]
fn cells_draw_side_by_side_with_marks_as_tall_as_their_row() {
    let (ui, editor) = table();
    let origin = line(&ui, &editor, "before");
    let (a1, b1) = (line(&ui, &editor, "A1"), line(&ui, &editor, "B1"));
    let (a2, b2) = (line(&ui, &editor, "A2"), line(&ui, &editor, "B2"));
    let after = line(&ui, &editor, "after");
    assert_eq!((a1.x, a1.width), (origin.x, 100.));
    assert_eq!((b1.x, b1.width), (origin.x + 110., 100.));
    assert_eq!(a1.y, origin.y + 20.);
    assert_eq!(b1.y, a1.y, "the row's cells share its top");
    assert!(a1.height >= 40., "the first cell wraps: {a1:?}");
    assert_eq!(b1.height, 20.);
    assert_eq!((a2.y, b2.y), (a1.y + a1.height, a1.y + a1.height));
    assert_eq!(after.y, a2.y + 20.);
    // Each cell's border: its own cell wide, its row high.
    let mut borders: Vec<Rect> = nodes(&ui, editor.node, |kind| match kind {
        NodeKind::Quad(quad) if quad.border_color == BORDER => Some(()),
        _ => None,
    })
    .into_iter()
    .map(|(_, at)| at)
    .collect();
    borders.sort_by(|a, b| (a.y, a.x).partial_cmp(&(b.y, b.x)).unwrap());
    assert_eq!(
        borders,
        [
            Rect::new(a1.x, a1.y, 100., a1.height),
            Rect::new(b1.x, a1.y, 100., a1.height),
            Rect::new(a1.x, a2.y, 100., 20.),
            Rect::new(b1.x, a2.y, 100., 20.),
        ]
    );
}

#[test]
fn up_and_down_move_between_rows_at_the_carets_x() {
    let (mut ui, editor) = table();
    let at = |needle: &str| TEXT.find(needle).unwrap();
    editor.set_selection(at("B1") + 1, at("B1") + 1);
    ui.prepare_frame();
    press(&mut ui, Key::ArrowDown);
    assert_eq!(
        editor.selection().1,
        at("B2") + 1,
        "the cell below, not the cell beside"
    );
    press(&mut ui, Key::ArrowDown);
    assert!(editor.selection().1 >= at("after"), "out of the table");
    press(&mut ui, Key::ArrowUp);
    press(&mut ui, Key::ArrowUp);
    assert_eq!(editor.selection().1, at("B1") + 1, "back up its column");
    press(&mut ui, Key::ArrowUp);
    assert!(
        editor.selection().1 < at("A1"),
        "to the line above the table"
    );
    // Down a wrapped cell's own rows first, then to the cell below it.
    editor.set_selection(at("A1"), at("A1"));
    ui.prepare_frame();
    press(&mut ui, Key::ArrowDown);
    let within = editor.selection().1;
    assert!(within > at("A1") && within < at("B1"), "{within}");
    for _ in 0..4 {
        press(&mut ui, Key::ArrowDown);
        if editor.selection().1 >= at("B1") {
            break;
        }
    }
    assert_eq!(editor.selection().1, at("A2"));
}

#[test]
fn a_marks_decoration_gives_its_quad_per_corner_radii_and_per_side_borders() {
    use std::sync::Arc;
    use zgx::decoration::{Corners, Decoration};
    use zgx::scene::Insets;
    let (ui, editor) = table();
    let decoration = Arc::new(Decoration {
        corners: Some(Corners {
            top_left: 15.,
            ..Corners::default()
        }),
        border_widths: Some(Insets {
            top: 1.,
            right: 1.,
            bottom: 0.,
            left: 1.,
        }),
        ..Decoration::default()
    });
    let shown = decoration.clone();
    editor.set_line_marks(Some(Rc::new(move |i| match cell(i) {
        Some((left, _)) if i == 1 => vec![LineMark {
            x: left,
            width: 100.,
            height: 20.,
            border_color: BORDER,
            border_width: 1.,
            decoration: Some(shown.clone()),
            ..LineMark::default()
        }],
        _ => Vec::new(),
    })));
    let ui = ui;
    ui.prepare_frame();
    let decorated: Vec<_> = nodes(&ui, editor.node, |kind| match kind {
        NodeKind::Quad(quad) if quad.border_color == BORDER => quad.decoration.clone(),
        _ => None,
    });
    assert_eq!(decorated.len(), 1, "the first cell's mark");
    assert_eq!(decorated[0].0, decoration);
}

const SPANNED: &str = "before\nSpan\nA\nB\nC\nafter";

/// A cell spanning three rows (`LineInset::spans_rows`, a table's `rowspan`), 120 tall with its room below, beside
/// three rows of one 20-tall cell each.
fn spanned() -> (Ui, EditorHandle) {
    let mut ui = Ui::new(500., 400.);
    let value = ui.signal(SPANNED.to_string());
    let view = ui.mount(
        text_area("Notes", value)
            .w(400.)
            .h(300.)
            .p(0.)
            .text_size(10.)
            .line_height(20.)
            .text_wrap(true),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_line_insets(Some(Rc::new(|i| match i {
        1 => LineInset {
            width: Some(100.),
            below: 100.,
            joins_next: true,
            spans_rows: 3,
            ..LineInset::default()
        },
        2..=4 => LineInset {
            left: 110.,
            width: Some(100.),
            ..LineInset::default()
        },
        _ => LineInset::default(),
    })));
    editor.set_line_marks(Some(Rc::new(|i| match i {
        1..=4 => vec![LineMark {
            x: if i == 1 { 0. } else { 110. },
            width: f32::INFINITY,
            height: f32::INFINITY,
            border_color: BORDER,
            border_width: 1.,
            ..LineMark::default()
        }],
        _ => Vec::new(),
    })));
    ui.input.focus(&ui.scene, Some(editor.node));
    ui.prepare_frame();
    (ui, editor)
}

/// The rows a cell spans share its height (equal rows, equally), its border reaches down them all, the line after
/// starts below them; Down from it leaves them all, and Down beside it goes row by row.
#[test]
fn a_cell_spanning_rows_reaches_down_them_and_they_share_its_height() {
    let (mut ui, editor) = spanned();
    let span = line(&ui, &editor, "Span");
    let (a, b, c) = (
        line(&ui, &editor, "A"),
        line(&ui, &editor, "B"),
        line(&ui, &editor, "C"),
    );
    let after = line(&ui, &editor, "after");
    assert_eq!(
        (a.y, a.x),
        (span.y, span.x + 110.),
        "its first row beside it"
    );
    assert_eq!(b.y, a.y + 40., "each row a third of its 120: {a:?} {b:?}");
    assert_eq!(c.y, b.y + 40.);
    assert_eq!(after.y, span.y + 120., "the line after below them all");
    let mut borders: Vec<Rect> = nodes(&ui, editor.node, |kind| match kind {
        NodeKind::Quad(quad) if quad.border_color == BORDER => Some(()),
        _ => None,
    })
    .into_iter()
    .map(|(_, at)| at)
    .collect();
    borders.sort_by(|p, q| (p.y, p.x).partial_cmp(&(q.y, q.x)).unwrap());
    assert_eq!(
        borders,
        [
            Rect::new(span.x, span.y, 100., 120.),
            Rect::new(a.x, a.y, 100., 40.),
            Rect::new(b.x, b.y, 100., 40.),
            Rect::new(c.x, c.y, 100., 40.),
        ]
    );
    let at = |needle: &str| SPANNED.find(needle).unwrap();
    editor.set_selection(at("A") + 1, at("A") + 1);
    ui.prepare_frame();
    press(&mut ui, Key::ArrowDown);
    assert_eq!(
        editor.selection().1,
        at("B") + 1,
        "the row below, beside it"
    );
    editor.set_selection(at("Span") + 2, at("Span") + 2);
    ui.prepare_frame();
    press(&mut ui, Key::ArrowDown);
    assert!(
        editor.selection().1 >= at("after"),
        "below all the rows it spans: {}",
        editor.selection().1
    );
}
