//! A table cell of several lines (`LineInset::stacks_next`): its lines stack inside the cell, the cells beside it keep
//! the row's top, and the row is as tall as its tallest cell.
use std::rc::Rc;
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    scene::{Color, NodeId, NodeKind, Rect},
    widgets::{EditorHandle, LineInset, LineMark, Ui},
};

const TEXT: &str = "before\nA1\nA1 again\nB1\nA2\nB2\nafter";
const BORDER: Color = Color(200, 0, 0, 255);

/// Each cell line's left, whether it stacks on the next, whether it joins the next, whether it starts its cell.
fn cell(i: usize) -> Option<(f32, bool, bool, bool)> {
    match i {
        1 => Some((0., true, false, true)),
        2 => Some((0., false, true, false)),
        4 => Some((0., false, true, true)),
        3 | 5 => Some((110., false, false, true)),
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
        Some((left, stacks_next, joins_next, _)) => LineInset {
            left,
            width: Some(100.),
            stacks_next,
            joins_next,
            ..LineInset::default()
        },
        None => LineInset::default(),
    })));
    editor.set_line_marks(Some(Rc::new(|i| match cell(i) {
        Some((left, _, _, true)) => vec![LineMark {
            x: left,
            width: f32::INFINITY,
            height: f32::INFINITY,
            border_color: BORDER,
            border_width: 1.,
            ..LineMark::default()
        }],
        _ => Vec::new(),
    })));
    ui.input.focus(&ui.scene, Some(view.node()));
    ui.prepare_frame();
    (ui, editor)
}
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
    nodes(ui, editor.node, |kind| match kind {
        NodeKind::Text { text, .. } => Some(text.to_string()),
        _ => None,
    })
    .into_iter()
    .find(|(shown, _)| shown == text)
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
fn at(needle: &str) -> usize {
    TEXT.find(needle).unwrap()
}

#[test]
fn a_cells_lines_stack_inside_it_and_the_row_is_as_tall_as_its_tallest_cell() {
    let (ui, editor) = table();
    let origin = line(&ui, &editor, "before");
    let (a1, again, b1) = (
        line(&ui, &editor, "A1"),
        line(&ui, &editor, "A1 again"),
        line(&ui, &editor, "B1"),
    );
    let (a2, b2) = (line(&ui, &editor, "A2"), line(&ui, &editor, "B2"));
    let after = line(&ui, &editor, "after");
    assert_eq!(a1.y, origin.y + 20.);
    assert_eq!(
        (again.x, again.y),
        (a1.x, a1.y + 20.),
        "under the cell's first line"
    );
    assert_eq!(
        (b1.x, b1.y),
        (origin.x + 110., a1.y),
        "the cell beside keeps the row's top"
    );
    assert_eq!(
        (a2.y, b2.y),
        (a1.y + 40., a1.y + 40.),
        "the next row under the taller cell"
    );
    assert_eq!(after.y, a2.y + 20.);
    let mut borders: Vec<Rect> = nodes(&ui, editor.node, |kind| match kind {
        NodeKind::Quad(quad) if quad.border_color == BORDER => Some(()),
        _ => None,
    })
    .into_iter()
    .map(|(_, r)| r)
    .collect();
    borders.sort_by(|a, b| (a.y, a.x).partial_cmp(&(b.y, b.x)).unwrap());
    assert_eq!(
        borders,
        [
            Rect::new(a1.x, a1.y, 100., 40.),
            Rect::new(b1.x, a1.y, 100., 40.),
            Rect::new(a1.x, a2.y, 100., 20.),
            Rect::new(b1.x, a2.y, 100., 20.),
        ],
        "each cell's border as tall as its row"
    );
}

#[test]
fn up_down_and_a_press_find_a_cells_stacked_lines() {
    let (mut ui, editor) = table();
    editor.set_selection(at("A1"), at("A1"));
    ui.prepare_frame();
    press(&mut ui, Key::ArrowDown);
    assert_eq!(editor.selection().1, at("A1 again"), "the cell's next line");
    press(&mut ui, Key::ArrowDown);
    assert_eq!(editor.selection().1, at("A2"), "then the cell below");
    press(&mut ui, Key::ArrowUp);
    assert_eq!(
        editor.selection().1,
        at("A1 again"),
        "up into the cell's last line"
    );
    press(&mut ui, Key::ArrowUp);
    assert_eq!(editor.selection().1, at("A1"));
    // a press under the cell beside's text, level with the stacked line: still that cell
    editor.set_selection(at("B1") + 2, at("B1") + 2);
    ui.prepare_frame();
    press(&mut ui, Key::ArrowDown);
    assert_eq!(
        editor.selection().1,
        at("B2") + 2,
        "B1 down to B2, past the stacked line beside it"
    );
    let again = line(&ui, &editor, "A1 again");
    let (x, y) = (again.x + 90., again.y + 10.);
    ui.dispatch(InputEvent::PointerDown {
        x,
        y,
        button: PointerButton::Primary,
    });
    ui.dispatch(InputEvent::PointerUp {
        x,
        y,
        button: PointerButton::Primary,
    });
    ui.prepare_frame();
    assert_eq!(
        editor.selection().1,
        at("A1 again") + "A1 again".len(),
        "a press beside the stacked line lands at its end"
    );
}

#[test]
fn an_edit_in_a_cell_keeps_the_stacked_rows() {
    let (mut ui, editor) = table();
    editor.set_selection(at("B1") + 2, at("B1") + 2);
    ui.prepare_frame();
    ui.dispatch(InputEvent::Text("!".into()));
    ui.prepare_frame();
    let (a1, a2) = (line(&ui, &editor, "A1"), line(&ui, &editor, "A2"));
    assert_eq!(line(&ui, &editor, "B1!").y, a1.y);
    assert_eq!(a2.y, a1.y + 40.);
}
