//! An app telling a text drop out of its own editor's selection
//! (`DraggedText::source_range`), to move it itself (keeping structure the
//! plain text drops): its `on_drop` on the editor runs before the editor's
//! own drop, and a drop it takes is its own: the editor neither moves the
//! text, nor deletes it as the drag ends, nor puts its selection back.
use std::{cell::RefCell, ops::Range, rc::Rc};
use zgx::{
    actions::Action,
    compose::prelude::*,
    input::{DraggedText, InputEvent, Modifiers, PointerButton},
    widgets::{EditorHandle, Ui},
};

/// Characters 6 points wide, 10 in from the field's edge; lines 20 high.
fn x_of(char: usize) -> f32 {
    10. + char as f32 * 6. + 1.
}
fn field(ui: &Ui, id: &str, text: &str) -> View {
    text_area(id, ui.signal(text.to_owned()))
        .id(id)
        .size(300., 60.)
        .p(10.)
        .text_size(10.)
        .line_height(20.)
}
/// The editor a hook mounted with it asks about, set once mounted.
type Slot = Rc<RefCell<Option<EditorHandle>>>;
/// What a hook heard: `source_range` of each drop it was asked about.
type Heard = Rc<RefCell<Vec<Option<Range<usize>>>>>;
/// `view` with an `on_drop_when` taking the text dragged out of the editor in
/// `slot` (or, with `all`, any text) and selecting 0..3 as its own move would.
fn hooked(view: View, slot: &Slot, heard: &Heard, all: bool) -> View {
    let (accepts, takes) = (slot.clone(), slot.clone());
    let hears = heard.clone();
    view.on_drop_when(
        move |text: &DraggedText| {
            let editor = accepts.borrow();
            all || text.source_range(editor.as_ref().unwrap()).is_some()
        },
        move |text: &DraggedText, _| {
            let editor = takes.borrow();
            let editor = editor.as_ref().unwrap();
            hears.borrow_mut().push(text.source_range(editor));
            editor.set_selection(0, 3);
        },
    )
}
fn editor(ui: &Ui, view: &zgx::compose::ViewHandle, id: &str) -> EditorHandle {
    ui.editor(view.find(id).unwrap()).unwrap()
}
fn pointer(ui: &mut Ui, event: InputEvent) {
    ui.dispatch_with_modifiers(event, Modifiers::default());
    ui.prepare_frame();
}
fn drag(ui: &mut Ui, from: (f32, f32), to: (f32, f32)) {
    let button = PointerButton::Primary;
    let (x, y) = from;
    pointer(ui, InputEvent::PointerDown { x, y, button });
    pointer(ui, InputEvent::PointerMove { x: x + 8., y });
    let (x, y) = to;
    pointer(ui, InputEvent::PointerMove { x, y });
    pointer(ui, InputEvent::PointerUp { x, y, button });
}

#[test]
fn a_drag_within_the_editor_gives_its_range_and_the_app_moves_it() {
    let mut ui = Ui::new(400., 300.);
    let (slot, heard): (Slot, Heard) = Default::default();
    let a = hooked(field(&ui, "a", "one two three"), &slot, &heard, false);
    let view = ui.mount(column().child(a));
    ui.prepare_frame();
    let a = editor(&ui, &view, "a");
    *slot.borrow_mut() = Some(a.clone());
    a.set_selection(4, 7);
    drag(&mut ui, (x_of(5), 20.), (x_of(13), 20.));
    assert_eq!(*heard.borrow(), vec![Some(4..7)]);
    assert_eq!(
        a.value().get(),
        "one two three",
        "the app's: neither moved by the editor nor deleted as the drag ended"
    );
    assert_eq!(a.selection(), (0, 3), "the selection the app set stays");
    assert!(!ui.input.is_dragging());
    // Nor does the editor keep the selection it had before that drag, to
    // put back as the next drag leaves it (the Leave after a drop taken
    // lets it go).
    let hover = Action::new(DraggedText::new(String::new()));
    assert!(ui.input.drag_over(&ui.scene, &hover, x_of(9), 20.));
    ui.prepare_frame();
    assert_eq!(a.selection(), (9, 9), "the caret where it would drop");
    ui.input.drag_out(&ui.scene);
    ui.prepare_frame();
    assert_eq!(a.selection(), (0, 3), "gone: the app's selection back");
}

#[test]
fn a_drag_from_another_editor_gives_none_and_drops_as_ever() {
    let mut ui = Ui::new(400., 300.);
    let (slot, heard): (Slot, Heard) = Default::default();
    let (a, b) = (field(&ui, "a", "alpha beta"), field(&ui, "b", "x"));
    let view = ui.mount(column().child(a).child(hooked(b, &slot, &heard, false)));
    ui.prepare_frame();
    let (a, b) = (editor(&ui, &view, "a"), editor(&ui, &view, "b"));
    *slot.borrow_mut() = Some(b.clone());
    a.set_selection(6, 10);
    drag(&mut ui, (x_of(7), 20.), (x_of(3), 80.));
    assert!(heard.borrow().is_empty(), "b's hook declines it");
    assert_eq!(b.value().get(), "xbeta", "b's own drop moves it");
    assert_eq!(a.value().get(), "alpha ");
}

#[test]
fn text_from_another_app_or_unowned_text_gives_none() {
    let mut ui = Ui::new(400., 300.);
    let (slot, heard): (Slot, Heard) = Default::default();
    let view = ui.mount(column().child(hooked(field(&ui, "a", "ab"), &slot, &heard, true)));
    ui.prepare_frame();
    let a = editor(&ui, &view, "a");
    *slot.borrow_mut() = Some(a.clone());
    assert_eq!(DraggedText::new("cd").source_range(&a), None);
    let dropped = Action::new(DraggedText::new("cd"));
    assert!(ui.input.drag_drop(&ui.scene, &dropped, x_of(2), 20.));
    ui.prepare_frame();
    assert_eq!(*heard.borrow(), vec![None]);
    assert_eq!(a.value().get(), "ab", "taken by the app: not inserted");
}

#[test]
fn a_drop_another_editors_app_takes_leaves_the_source_text() {
    let mut ui = Ui::new(400., 300.);
    let (slot, heard): (Slot, Heard) = Default::default();
    let (a, b) = (field(&ui, "a", "alpha beta"), field(&ui, "b", "x"));
    let view = ui.mount(column().child(a).child(hooked(b, &slot, &heard, true)));
    ui.prepare_frame();
    let (a, b) = (editor(&ui, &view, "a"), editor(&ui, &view, "b"));
    *slot.borrow_mut() = Some(b.clone());
    a.set_selection(6, 10);
    drag(&mut ui, (x_of(7), 20.), (x_of(3), 80.));
    assert_eq!(*heard.borrow(), vec![None], "not b's own text");
    assert_eq!(b.value().get(), "x");
    assert_eq!(a.value().get(), "alpha beta", "the source's text stays");
}

#[test]
fn a_drop_copies_by_the_platforms_modifier() {
    let option = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    let control = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    assert_eq!(option.drag_copies(), cfg!(target_os = "macos"));
    assert_eq!(control.drag_copies(), !cfg!(target_os = "macos"));
    assert!(!Modifiers::default().drag_copies());
}
