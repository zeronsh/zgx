//! What had the keyboard, and a field's selection, taken and put back
//! (`Ui::focus_snapshot`, `Ui::restore_focus`): a guide that takes the user
//! elsewhere returns them where they were, nothing scrolled.
use std::{cell::Cell, rc::Rc};
use zgx::{
    compose::{RowAlign, ViewHandle, prelude::*},
    reactive::Signal,
    scene::NodeId,
    widgets::{FocusSnapshot, Ui},
};

thread_local! {
    /// The fixture's root, to find its nodes by id.
    static ROOT: std::cell::RefCell<Option<ViewHandle>> = const { std::cell::RefCell::new(None) };
}

fn settle(ui: &mut Ui) {
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
}

/// A field, a textarea, a code editor (lines coloured by a highlighter)
/// and a button, the code editor shown while `shown` says.
fn fixture() -> (Ui, Signal<String>, Signal<bool>) {
    let mut ui = Ui::new(400., 400.);
    let field = ui.signal("hello wide world".to_string());
    let area = ui.signal(["line"; 100].join("\n"));
    let code = ui.signal("local x = 1\nlocal y = 2".to_string());
    let shown = ui.signal(true);
    let code_view = switch_or_none(
        {
            let shown = shown.clone();
            move || shown.get()
        },
        move |shown, _| {
            shown.then(|| {
                text_area("Code", code.clone())
                    .id("code")
                    .highlighter(|_line, open| (Default::default(), open))
                    .size(200., 60.)
            })
        },
    );
    let root = ui.mount(
        column()
            .child(
                text_input("Field", field.clone())
                    .id("field")
                    .size(200., 24.),
            )
            .child(
                text_area("Area", area)
                    .id("area")
                    .size(200., 40.)
                    .text_size(10.)
                    .line_height(20.),
            )
            .child(code_view)
            .child(button().id("other").size(80., 30.).child(text("Other"))),
    );
    ROOT.with(|it| *it.borrow_mut() = Some(root));
    settle(&mut ui);
    (ui, field, shown)
}

fn node(_: &Ui, id: &str) -> NodeId {
    ROOT.with(|it| it.borrow().as_ref().unwrap().find(id).unwrap())
}

/// Focus `id`, select `anchor..focus` in it, and take a snapshot; then
/// focus the button and move the field's selection elsewhere.
fn take(ui: &mut Ui, id: &str, anchor: usize, focus: usize) -> FocusSnapshot {
    let field = node(ui, id);
    ui.input.focus(&ui.scene, Some(field));
    let editor = ui.editor(field).unwrap();
    editor.set_selection(anchor, focus);
    settle(ui);
    let snapshot = ui.focus_snapshot().unwrap();
    assert_eq!(snapshot.node, field);
    assert_eq!(snapshot.selection, Some((anchor, focus)));
    let other = node(ui, "other");
    ui.input.focus(&ui.scene, Some(other));
    editor.set_selection(0, 0);
    settle(ui);
    snapshot
}

#[test]
fn each_editor_kind_gets_its_focus_and_selection_back() {
    for (id, anchor, focus) in [("field", 12, 6), ("area", 3, 30), ("code", 6, 11)] {
        let (mut ui, ..) = fixture();
        let snapshot = take(&mut ui, id, anchor, focus);
        assert_ne!(ui.input.focused(), Some(snapshot.node), "{id}");
        assert!(ui.restore_focus(&snapshot), "{id}");
        settle(&mut ui);
        assert_eq!(ui.input.focused(), Some(snapshot.node), "{id}");
        let editor = ui.editor(snapshot.node).unwrap();
        assert_eq!(
            editor.selection(),
            (anchor, focus),
            "{id}: anchor and caret"
        );
        assert_eq!(ui.focus_snapshot(), Some(snapshot), "{id}");
    }
}

#[test]
fn a_button_comes_back_without_a_selection() {
    let (mut ui, ..) = fixture();
    let other = node(&ui, "other");
    ui.input.focus(&ui.scene, Some(other));
    let snapshot = ui.focus_snapshot().unwrap();
    assert_eq!(snapshot.selection, None);
    ui.input.focus(&ui.scene, None);
    assert_eq!(ui.focus_snapshot(), None, "nothing has focus");
    assert!(ui.restore_focus(&snapshot));
    assert_eq!(ui.input.focused(), Some(other));
}

#[test]
fn an_unmounted_field_is_not_restored() {
    let (mut ui, _, shown) = fixture();
    let snapshot = take(&mut ui, "code", 0, 5);
    let before = ui.input.focused();
    shown.set(false);
    settle(&mut ui);
    assert!(!ui.restore_focus(&snapshot));
    assert_eq!(ui.input.focused(), before, "focus stays where it was");
}

#[test]
fn offsets_past_the_text_are_not_restored() {
    let (mut ui, field, _) = fixture();
    let snapshot = take(&mut ui, "field", 16, 16);
    let before = ui.input.focused();
    field.set("hi".to_string());
    settle(&mut ui);
    assert!(!ui.restore_focus(&snapshot));
    assert_eq!(ui.input.focused(), before, "focus stays where it was");
    // Inside a character is no better than past the end.
    field.set("héllo wide world".to_string());
    settle(&mut ui);
    let inside = FocusSnapshot {
        selection: Some((2, 2)),
        ..snapshot
    };
    assert!(!ui.restore_focus(&inside));
}

#[test]
fn restoring_scrolls_nothing() {
    let (mut ui, ..) = fixture();
    let area = node(&ui, "area");
    let editor = ui.editor(area).unwrap();
    let top = Rc::new(Cell::new(usize::MAX));
    let told = top.clone();
    editor.on_top_line(move |line| told.set(line));
    // Line 90 selected brings it into view, as a field does.
    let at = "line\n".len() * 90;
    ui.input.focus(&ui.scene, Some(area));
    editor.set_selection(at, at + 2);
    settle(&mut ui);
    let snapshot = ui.focus_snapshot().unwrap();
    assert!(
        top.get() > 80,
        "selecting scrolled to line 90: {}",
        top.get()
    );
    // The user scrolls back up and goes elsewhere, the selection kept.
    editor.scroll_to_line(0, RowAlign::Start);
    let other = node(&ui, "other");
    ui.input.focus(&ui.scene, Some(other));
    settle(&mut ui);
    let scrolled = top.get();
    assert!(scrolled < 10, "{scrolled}");
    assert!(ui.restore_focus(&snapshot));
    settle(&mut ui);
    assert_eq!(ui.input.focused(), Some(area));
    assert_eq!(editor.selection(), (at, at + 2));
    assert_eq!(
        top.get(),
        scrolled,
        "the textarea stays where the user left it"
    );
}
