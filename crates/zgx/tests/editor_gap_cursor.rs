//! A text area's gap cursor (`EditorHandle::set_gaps`): a caret between
//! lines that hold no text to put one in, which the arrows and a press go
//! into and out of, drawn as a bar along the boundary, and what is typed at
//! it goes to the app.
use std::{cell::RefCell, rc::Rc};
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    widgets::{EditorHandle, Ui},
};

/// "a", two lines a table would fill, "b": lines 0 to 3.
const TEXT: &str = "a\n\n\nb";

type Told = Rc<RefCell<Vec<String>>>;

/// A text area of `TEXT` whose gaps are `allowed`, and what it told: gaps
/// gone into and out of, and what was typed at them.
fn fixture(allowed: &'static [usize]) -> (Ui, EditorHandle, Told) {
    let mut ui = Ui::new(400., 300.);
    let view = ui.mount(
        text_area("Note", ui.signal(TEXT.to_owned()))
            .size(200., 160.)
            .p(10.)
            .text_size(10.)
            .line_height(20.),
    );
    ui.prepare_frame();
    ui.input.focus(&ui.scene, Some(view.node()));
    let editor = ui.focused_editor().unwrap();
    let told: Told = Rc::default();
    editor.set_gaps(Some(Rc::new(move |n| allowed.contains(&n))));
    let heard = told.clone();
    editor.on_gap(move |at| heard.borrow_mut().push(format!("gap {at:?}")));
    let heard = told.clone();
    editor.on_gap_input(move |at, input| heard.borrow_mut().push(format!("{at} {input:?}")));
    editor.editor.borrow_mut().set_selection(0, 0);
    editor.refresh();
    ui.prepare_frame();
    (ui, editor, told)
}
fn key(ui: &mut Ui, key: Key) {
    ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    ui.prepare_frame();
}
fn caret(ui: &Ui, editor: &EditorHandle) -> zgx::geometry::Rect {
    ui.scene.borrow().bounds(editor.caret)
}
fn focus(editor: &EditorHandle) -> usize {
    editor.editor.borrow().selection().focus
}
fn press(ui: &mut Ui, x: f32, y: f32) {
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
}

/// → at a line's end goes into the gap after it, drawn as a bar along the
/// boundary; → again goes on into the next line, ← back.
#[test]
fn the_arrows_go_into_a_gap_and_out_of_it() {
    let (mut ui, editor, told) = fixture(&[1, 2, 3]);
    let text_caret = caret(&ui, &editor);
    editor.set_selection(1, 1);
    ui.prepare_frame();
    key(&mut ui, Key::ArrowRight);
    assert_eq!(editor.gap(), Some(1));
    let bar = caret(&ui, &editor);
    assert_eq!((bar.width, bar.height), (20., 2.), "{bar:?}");
    // Along the top of line 1, a line below the first's.
    assert_eq!(bar.y + 1., text_caret.y + 20., "{bar:?} {text_caret:?}");
    key(&mut ui, Key::ArrowRight);
    assert_eq!(editor.gap(), None);
    assert_eq!(focus(&editor), 2, "the start of line 1");
    assert_eq!(caret(&ui, &editor).width, 1.);
    key(&mut ui, Key::ArrowLeft);
    assert_eq!(editor.gap(), Some(1));
    key(&mut ui, Key::ArrowLeft);
    assert_eq!((editor.gap(), focus(&editor)), (None, 1));
    // ↑ from a line's first row, and ↓ from its last.
    editor.set_selection(5, 5);
    ui.prepare_frame();
    key(&mut ui, Key::ArrowUp);
    assert_eq!(editor.gap(), Some(3));
    key(&mut ui, Key::ArrowDown);
    assert_eq!((editor.gap(), focus(&editor)), (None, 4));
    assert_eq!(
        *told.borrow(),
        [
            "gap Some(1)",
            "gap None",
            "gap Some(1)",
            "gap None",
            "gap Some(3)",
            "gap None"
        ]
    );
}

/// At the very start and end, a gap holds the arrows that would leave the
/// text.
#[test]
fn a_gap_may_sit_at_the_texts_start_and_end() {
    let (mut ui, editor, _) = fixture(&[0, 4]);
    key(&mut ui, Key::ArrowLeft);
    assert_eq!(editor.gap(), Some(0));
    key(&mut ui, Key::ArrowUp);
    assert_eq!(editor.gap(), Some(0), "nothing before it");
    editor.set_selection(5, 5);
    ui.prepare_frame();
    key(&mut ui, Key::ArrowDown);
    assert_eq!(editor.gap(), Some(4));
    let bar = caret(&ui, &editor);
    editor.set_selection(5, 5);
    ui.prepare_frame();
    assert!(bar.y > caret(&ui, &editor).y, "below the last line");
}

/// What is typed at a gap goes to the app, not into the text.
#[test]
fn what_is_typed_at_a_gap_goes_to_the_app() {
    let (mut ui, editor, told) = fixture(&[2]);
    editor.set_gap(Some(2));
    ui.prepare_frame();
    ui.dispatch(InputEvent::Text("x".into()));
    key(&mut ui, Key::Enter);
    key(&mut ui, Key::Backspace);
    key(&mut ui, Key::Delete);
    editor.paste("pasted");
    assert_eq!(editor.editor.borrow().text(), TEXT);
    assert_eq!(
        *told.borrow(),
        [
            "gap Some(2)",
            r#"2 Text("x")"#,
            "2 Enter",
            "2 Delete { backward: true }",
            "2 Delete { backward: false }",
            r#"2 Paste("pasted")"#,
        ]
    );
    // The app put a paragraph there and the caret in it.
    editor.set_selection(3, 3);
    assert_eq!(editor.gap(), None);
}

/// Only where the app allows: elsewhere the arrows go as they always do.
#[test]
fn a_boundary_the_app_doesnt_allow_is_passed_by() {
    let (mut ui, editor, told) = fixture(&[1]);
    editor.set_selection(5, 5);
    ui.prepare_frame();
    key(&mut ui, Key::ArrowLeft);
    assert_eq!((editor.gap(), focus(&editor)), (None, 4));
    key(&mut ui, Key::ArrowUp);
    assert_eq!((editor.gap(), focus(&editor)), (None, 3));
    assert!(told.borrow().is_empty());
    // With gaps taken away, the caret leaves the one it was in.
    editor.set_gap(Some(1));
    editor.set_gaps(None);
    assert_eq!(editor.gap(), None);
}

/// A press on a boundary the app allows puts the caret in its gap; one in
/// the text, back in the text.
#[test]
fn a_press_on_an_allowed_boundary_goes_into_its_gap() {
    let (mut ui, editor, _) = fixture(&[1]);
    editor.set_gap(Some(1));
    ui.prepare_frame();
    let bar = caret(&ui, &editor);
    editor.set_gap(None);
    ui.prepare_frame();
    press(&mut ui, bar.x + 30., bar.y + 1.);
    assert_eq!(editor.gap(), Some(1));
    // Below it, on the boundary before line 2, which isn't allowed: a press
    // in the text.
    press(&mut ui, bar.x + 30., bar.y + 21.);
    assert_eq!(editor.gap(), None);
    // In the middle of line 0's row: the text.
    editor.set_gap(Some(1));
    ui.prepare_frame();
    press(&mut ui, bar.x + 3., bar.y - 10.);
    assert_eq!((editor.gap(), focus(&editor)), (None, 0));
}

/// A text area that sets no gaps goes as it did.
#[test]
fn without_gaps_the_arrows_go_as_ever() {
    let mut ui = Ui::new(400., 300.);
    let view = ui.mount(
        text_area("Note", ui.signal(TEXT.to_owned()))
            .size(200., 160.)
            .p(10.)
            .text_size(10.)
            .line_height(20.),
    );
    ui.prepare_frame();
    ui.input.focus(&ui.scene, Some(view.node()));
    let editor = ui.focused_editor().unwrap();
    editor.editor.borrow_mut().set_selection(1, 1);
    editor.refresh();
    key(&mut ui, Key::ArrowRight);
    assert_eq!((editor.gap(), focus(&editor)), (None, 2));
    ui.dispatch(InputEvent::Text("x".into()));
    assert_eq!(editor.editor.borrow().text(), "a\nx\n\nb");
}
