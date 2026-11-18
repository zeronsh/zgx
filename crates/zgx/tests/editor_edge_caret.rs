//! A caret at an inline span's edge (`EditorHandle::set_caret_edges`): one
//! offset, inside the span or outside it; the first arrow there crosses
//! the edge without moving over a character, and the caret shows a hair
//! inside or outside.
use std::rc::Rc;
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers},
    widgets::{CaretSide, EditorHandle, Ui},
};

/// `ab`, code `cd`, `ef`: the span's edges at 2 and 4.
fn fixture(edges: bool) -> (Ui, EditorHandle) {
    let mut ui = Ui::new(400., 300.);
    let view = ui.mount(
        text_area("Note", ui.signal("abcdef".to_owned()))
            .size(200., 100.)
            .p(10.)
            .text_size(10.),
    );
    ui.prepare_frame();
    ui.input.focus(&ui.scene, Some(view.node()));
    let editor = ui.focused_editor().unwrap();
    if edges {
        editor.set_caret_edges(Some(Rc::new(|at| (2..=4).contains(&at).then_some(2..4))));
    }
    editor.set_selection(1, 1);
    ui.prepare_frame();
    (ui, editor)
}
fn key(ui: &mut Ui, key: Key) {
    ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    ui.prepare_frame();
}
fn at(ui: &Ui, editor: &EditorHandle) -> (usize, CaretSide, f32) {
    (
        editor.editor.borrow().selection().focus,
        editor.caret_side(),
        ui.scene.borrow().bounds(editor.caret).x,
    )
}

#[test]
fn the_first_arrow_at_an_edge_crosses_it_without_moving() {
    let (mut ui, editor) = fixture(true);
    let (mut plain, plain_editor) = fixture(false);
    let laid_out = |offset: usize, plain: &mut Ui| {
        plain_editor.set_selection(offset, offset);
        plain.prepare_frame();
        plain.scene.borrow().bounds(plain_editor.caret).x
    };
    let (start, end) = (laid_out(2, &mut plain), laid_out(4, &mut plain));
    // To the span's start from outside: outside, a hair before it.
    key(&mut ui, Key::ArrowRight);
    assert_eq!(at(&ui, &editor), (2, CaretSide::Outside, start - 1.));
    // → crosses into it, not moving over `c`.
    key(&mut ui, Key::ArrowRight);
    assert_eq!(at(&ui, &editor), (2, CaretSide::Inside, start + 1.));
    key(&mut ui, Key::ArrowRight);
    key(&mut ui, Key::ArrowRight);
    // Its end, reached from within: inside; → crosses out, then moves.
    assert_eq!(at(&ui, &editor), (4, CaretSide::Inside, end - 1.));
    key(&mut ui, Key::ArrowRight);
    assert_eq!(at(&ui, &editor), (4, CaretSide::Outside, end + 1.));
    key(&mut ui, Key::ArrowRight);
    assert_eq!(at(&ui, &editor).0, 5);
    // ← back: outside at the end, then in, then over `d`.
    key(&mut ui, Key::ArrowLeft);
    assert_eq!(at(&ui, &editor), (4, CaretSide::Outside, end + 1.));
    key(&mut ui, Key::ArrowLeft);
    assert_eq!(at(&ui, &editor), (4, CaretSide::Inside, end - 1.));
    key(&mut ui, Key::ArrowLeft);
    assert_eq!(at(&ui, &editor).0, 3);
}

/// The app sets the side (its stored marks); a selection set or a press
/// puts it back outside; away from an edge the caret shows where laid out.
#[test]
fn the_app_sets_and_reads_the_side() {
    let (mut ui, editor) = fixture(true);
    editor.set_selection(4, 4);
    editor.set_caret_side(CaretSide::Inside);
    ui.prepare_frame();
    assert_eq!(editor.caret_side(), CaretSide::Inside);
    let inside = ui.scene.borrow().bounds(editor.caret).x;
    editor.set_selection(4, 4);
    ui.prepare_frame();
    assert_eq!(editor.caret_side(), CaretSide::Outside);
    assert_eq!(ui.scene.borrow().bounds(editor.caret).x, inside + 2.);
    ui.dispatch(InputEvent::Text("x".into()));
    assert_eq!(editor.caret_side(), CaretSide::Outside);
}

/// Without edges, the arrows move over each character as ever.
#[test]
fn without_edges_the_arrows_move_as_ever() {
    let (mut ui, editor) = fixture(false);
    for expected in [2, 3, 4, 5] {
        key(&mut ui, Key::ArrowRight);
        assert_eq!(at(&ui, &editor).0, expected);
        assert_eq!(editor.caret_side(), CaretSide::Outside);
    }
}
