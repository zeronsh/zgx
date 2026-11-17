//! Selected text drags as AppKit's text views and browsers drag it: within
//! an editor it moves (one undo step), into another it moves too (Option on
//! macOS, Control elsewhere, copies), out of the window as plain text, and a
//! press on the selection that does not move puts the caret there. Text
//! another app drags in drops where it was over, its text known only then.
use zgx::{
    actions::Action,
    compose::prelude::*,
    input::{DragItem, DraggedText, InputEvent, Modifiers, PointerButton},
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
fn editor(ui: &Ui, view: &zgx::compose::ViewHandle, id: &str) -> EditorHandle {
    ui.editor(view.find(id).unwrap()).unwrap()
}
fn pointer(ui: &mut Ui, event: InputEvent, modifiers: Modifiers) {
    ui.dispatch_with_modifiers(event, modifiers);
    ui.prepare_frame();
}
fn drag(ui: &mut Ui, from: (f32, f32), to: (f32, f32), modifiers: Modifiers) {
    let button = PointerButton::Primary;
    pointer(
        ui,
        InputEvent::PointerDown {
            x: from.0,
            y: from.1,
            button,
        },
        Modifiers::default(),
    );
    pointer(
        ui,
        InputEvent::PointerMove {
            x: from.0 + 8.,
            y: from.1,
        },
        Modifiers::default(),
    );
    pointer(ui, InputEvent::PointerMove { x: to.0, y: to.1 }, modifiers);
    pointer(
        ui,
        InputEvent::PointerUp {
            x: to.0,
            y: to.1,
            button,
        },
        modifiers,
    );
}
fn select(editor: &EditorHandle, range: std::ops::Range<usize>) {
    editor
        .editor
        .borrow_mut()
        .set_selection(range.start, range.end);
    editor.refresh();
}

#[test]
fn a_selection_dragged_within_an_editor_moves_in_one_undo_step() {
    let mut ui = Ui::new(400., 300.);
    let a = field(&ui, "a", "one two three");
    let view = ui.mount(column().child(a));
    ui.prepare_frame();
    let a = editor(&ui, &view, "a");
    select(&a, 4..7);
    drag(
        &mut ui,
        (x_of(5), 20.),
        (x_of(13), 20.),
        Modifiers::default(),
    );
    assert_eq!(a.value().get(), "one  threetwo");
    assert_eq!(a.copy(), "two", "the moved text selected");
    assert!(a.editor.borrow_mut().undo());
    assert_eq!(
        a.editor.borrow().text(),
        "one two three",
        "one step puts it back"
    );
}

#[test]
fn a_press_on_the_selection_let_go_puts_the_caret_there() {
    let mut ui = Ui::new(400., 300.);
    let a = field(&ui, "a", "one two three");
    let view = ui.mount(column().child(a));
    ui.prepare_frame();
    let a = editor(&ui, &view, "a");
    select(&a, 4..7);
    let button = PointerButton::Primary;
    pointer(
        &mut ui,
        InputEvent::PointerDown {
            x: x_of(5),
            y: 20.,
            button,
        },
        Modifiers::default(),
    );
    assert_eq!(a.selection(), (4, 7), "the selection stays while pressed");
    pointer(
        &mut ui,
        InputEvent::PointerUp {
            x: x_of(5),
            y: 20.,
            button,
        },
        Modifiers::default(),
    );
    assert_eq!(a.selection(), (5, 5));
    assert_eq!(a.value().get(), "one two three");
}

#[test]
fn text_dragged_into_another_editor_moves_unless_copied_and_leaves_as_text() {
    for copy in [false, true] {
        let mut ui = Ui::new(400., 300.);
        let (a, b) = (field(&ui, "a", "alpha beta"), field(&ui, "b", "x"));
        let view = ui.mount(column().child(a).child(b));
        ui.prepare_frame();
        let (a, b) = (editor(&ui, &view, "a"), editor(&ui, &view, "b"));
        select(&a, 6..10);
        let button = PointerButton::Primary;
        pointer(
            &mut ui,
            InputEvent::PointerDown {
                x: x_of(7),
                y: 20.,
                button,
            },
            Modifiers::default(),
        );
        pointer(
            &mut ui,
            InputEvent::PointerMove {
                x: x_of(7) + 8.,
                y: 20.,
            },
            Modifiers::default(),
        );
        // As it starts: what other apps would take, plain text.
        let state = ui.input.drag_state().expect("dragging");
        assert_eq!(
            state.payload.downcast_ref::<DraggedText>().unwrap().text(),
            "beta"
        );
        assert_eq!(
            state.export.as_deref(),
            Some(&[DragItem::Text("beta".into())][..])
        );
        let modifiers = Modifiers {
            alt: copy && cfg!(target_os = "macos"),
            control: copy && !cfg!(target_os = "macos"),
            ..Modifiers::default()
        };
        // Onto the second field (60 high, below the first), past its "x".
        pointer(
            &mut ui,
            InputEvent::PointerMove { x: x_of(3), y: 80. },
            modifiers,
        );
        pointer(
            &mut ui,
            InputEvent::PointerUp {
                x: x_of(3),
                y: 80.,
                button,
            },
            modifiers,
        );
        assert_eq!(b.value().get(), "xbeta", "copy {copy}");
        assert_eq!(b.copy(), "beta", "the dropped text selected");
        let left = if copy { "alpha beta" } else { "alpha " };
        assert_eq!(a.value().get(), left, "copy {copy}");
    }
}

#[test]
fn text_from_another_app_drops_where_it_was_over_focused_and_selected() {
    let mut ui = Ui::new(400., 300.);
    let (a, b, c) = (
        field(&ui, "a", "alpha beta"),
        field(&ui, "b", "x"),
        field(&ui, "c", "kept").read_only(true),
    );
    let view = ui.mount(column().child(a).child(b).child(c));
    ui.prepare_frame();
    let (a, b) = (editor(&ui, &view, "a"), editor(&ui, &view, "b"));
    ui.input.focus(&ui.scene, view.find("a"));
    select(&b, 0..1);
    // The platform names text while it is over: what it is waits for the drop.
    let hover = Action::new(DraggedText::new(String::new()));
    assert!(ui.input.drag_over(&ui.scene, &hover, x_of(3), 80.));
    ui.prepare_frame();
    assert_eq!(b.selection(), (1, 1), "the caret where it would drop");
    ui.input.drag_out(&ui.scene);
    ui.prepare_frame();
    assert_eq!(b.selection(), (0, 1), "gone: the selection back");
    assert!(
        !ui.input.drag_over(&ui.scene, &hover, x_of(1), 140.),
        "a read-only field takes none"
    );
    assert!(ui.input.drag_over(&ui.scene, &hover, x_of(3), 80.));
    let dropped = Action::new(DraggedText::new("dropped"));
    assert!(ui.input.drag_drop(&ui.scene, &dropped, x_of(3), 80.));
    ui.prepare_frame();
    assert_eq!(b.value().get(), "xdropped");
    assert_eq!(b.copy(), "dropped", "the dropped text selected");
    assert_eq!(ui.input.focused(), view.find("b"), "and its field focused");
    assert_eq!(a.value().get(), "alpha beta");
    // A field selecting all as it is focused keeps the dropped text selected.
    let mut ui = Ui::new(400., 300.);
    let view = ui.mount(column().child(field(&ui, "a", "ab").select_on_focus(true)));
    ui.prepare_frame();
    let a = editor(&ui, &view, "a");
    let dropped = Action::new(DraggedText::new("cd"));
    assert!(ui.input.drag_drop(&ui.scene, &dropped, x_of(2), 20.));
    ui.prepare_frame();
    assert_eq!(ui.input.focused(), view.find("a"));
    assert_eq!((a.value().get(), a.copy()), ("abcd".into(), "cd".into()));
}

#[test]
fn a_read_only_editor_takes_no_drop_and_a_password_drags_nothing() {
    let mut ui = Ui::new(400., 300.);
    let (a, b) = (
        field(&ui, "a", "alpha beta"),
        field(&ui, "b", "x").read_only(true),
    );
    let view = ui.mount(column().child(a).child(b));
    ui.prepare_frame();
    let (a, b) = (editor(&ui, &view, "a"), editor(&ui, &view, "b"));
    select(&a, 6..10);
    drag(
        &mut ui,
        (x_of(7), 20.),
        (x_of(3), 80.),
        Modifiers::default(),
    );
    assert_eq!(b.value().get(), "x");
    assert_eq!(a.value().get(), "alpha beta", "not taken: it stays");
    let mut ui = Ui::new(400., 300.);
    let secret = ui.signal(String::from("hunter2"));
    let view = ui.mount(
        column().child(
            text_input("Password", secret)
                .secure(true)
                .id("p")
                .size(300., 40.),
        ),
    );
    ui.prepare_frame();
    let p = editor(&ui, &view, "p");
    select(&p, 0..7);
    let button = PointerButton::Primary;
    pointer(
        &mut ui,
        InputEvent::PointerDown {
            x: 20.,
            y: 20.,
            button,
        },
        Modifiers::default(),
    );
    pointer(
        &mut ui,
        InputEvent::PointerMove { x: 60., y: 20. },
        Modifiers::default(),
    );
    assert!(!ui.input.is_dragging(), "a password never leaves its field");
}

/// A text area with `drop_caret(false)` takes dragged text as any does, and
/// drops it where it is let go, but its caret and selection stay where they
/// were while the drag is over it: the app shows where it would drop. One
/// without moves its caret there, as before.
#[test]
fn a_text_area_without_a_drop_caret_keeps_its_selection_while_text_is_over_it() {
    for shows in [false, true] {
        let mut ui = Ui::new(400., 300.);
        let ended = std::rc::Rc::new(std::cell::Cell::new(None));
        let told = ended.clone();
        let source = div()
            .size(100., 30.)
            .on_drag(|| DraggedText::new("XY"))
            .on_drag_end(move |_: &DraggedText, accepted| told.set(Some(accepted)));
        let b = field(&ui, "b", "one two three").drop_caret(shows);
        let view = ui.mount(column().child(source).child(b));
        ui.prepare_frame();
        let b = editor(&ui, &view, "b");
        select(&b, 0..3);
        let button = PointerButton::Primary;
        let at = (x_of(8), 50.);
        for event in [
            InputEvent::PointerDown {
                x: 10.,
                y: 10.,
                button,
            },
            InputEvent::PointerMove { x: 30., y: 10. },
            InputEvent::PointerMove { x: at.0, y: at.1 },
        ] {
            pointer(&mut ui, event, Modifiers::default());
        }
        let selection = b.selection();
        if shows {
            assert_eq!(selection, (8, 8), "the caret where it drops");
        } else {
            assert_eq!(selection, (0, 3), "kept while it is over it");
        }
        assert_eq!(b.offset_at(at.0, at.1), Some(8));
        pointer(
            &mut ui,
            InputEvent::PointerUp {
                x: at.0,
                y: at.1,
                button,
            },
            Modifiers::default(),
        );
        assert_eq!(b.value().get(), "one two XYthree", "dropped where let go");
        assert_eq!(ended.get(), Some(true), "taken");
    }
}
