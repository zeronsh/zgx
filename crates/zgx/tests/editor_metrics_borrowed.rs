//! `EditorHandle::text_metrics` asked again, at the width asked before, by
//! an app's callback that runs while a key, a press or a drag moves the caret,
//! or its atoms are followed (the editor's model is borrowed then): it
//! answers what it kept.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    widgets::{EditorHandle, Ui},
};

/// A callback's: the handle once there, whether to ask (once), the answers.
#[derive(Clone, Default)]
struct Asker {
    handle: Rc<RefCell<Option<EditorHandle>>>,
    ask: Rc<Cell<bool>>,
    got: Rc<RefCell<Vec<Option<f32>>>>,
    width: f32,
}
impl Asker {
    fn new(width: f32) -> Self {
        Self {
            width,
            ..Self::default()
        }
    }
    fn run(&self) {
        if self.ask.take() {
            let editor = self.handle.borrow().clone().unwrap();
            let got = editor.text_metrics(self.width).map(|m| m.wrapped_height);
            self.got.borrow_mut().push(got);
        }
    }
}
/// Sixty lines, five of them in view, measured once at 200.
fn note(asker: &Asker, change: impl FnOnce(&Ui, View) -> View) -> (Ui, EditorHandle) {
    let mut ui = Ui::new(600., 400.);
    let text: Vec<String> = (0..60).map(|i| format!("line {i}")).collect();
    let value = ui.signal(text.join("\n"));
    let area = text_area("Note", value)
        .w(200.)
        .h(100.)
        .p(0.)
        .text_size(10.)
        .line_height(20.)
        .text_wrap(true);
    let area = change(&ui, area);
    let view = ui.mount(area);
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_selection(0, 0);
    ui.input.focus(&ui.scene, Some(view.node()));
    for _ in 0..3 {
        ui.prepare_frame();
    }
    assert_eq!(editor.text_metrics(200.).unwrap().wrapped_height, 1200.);
    *asker.handle.borrow_mut() = Some(editor.clone());
    (ui, editor)
}
fn key(ui: &mut Ui, key: Key) {
    ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers: Modifiers::default(),
        repeat: false,
    });
}
/// The note's lines ask `asker` for their room.
fn in_room(asker: &Asker) -> (Ui, EditorHandle) {
    let asked = asker.clone();
    note(asker, |_, area| {
        area.line_insets(move |_, _: &str, _| {
            asked.run();
            zgx::widgets::LineInset::default()
        })
    })
}
/// Page Down lays out lines not seen yet, each asked for its room.
fn by_key(width: f32, want: Option<f32>) {
    let asker = Asker::new(width);
    let (mut ui, editor) = in_room(&asker);
    asker.ask.set(true);
    key(&mut ui, Key::PageDown);
    assert_eq!(*asker.got.borrow(), [want]);
    assert!(editor.selection().1 > 0, "moved a page");
    *asker.handle.borrow_mut() = None;
}
#[test]
fn line_insets_a_key_runs_are_answered() {
    by_key(200., Some(1200.));
}
/// A width not kept cannot be laid out then: none, never a panic.
#[test]
fn line_insets_a_key_runs_are_told_nothing_of_another_width() {
    by_key(90., None);
}
/// An arrow asks whether the caret is at a span's edge.
#[test]
fn caret_edges_a_key_asks_for_are_answered() {
    let asker = Asker::new(200.);
    let (mut ui, editor) = note(&asker, |_, area| area);
    let asked = asker.clone();
    editor.set_caret_edges(Some(Rc::new(move |_| {
        asked.run();
        None
    })));
    asker.ask.set(true);
    key(&mut ui, Key::ArrowRight);
    assert_eq!(*asker.got.borrow(), [Some(1200.)]);
    *asker.handle.borrow_mut() = None;
}
/// A press past the lines in view (a captured pointer's) lays out those it
/// is at.
fn press(ui: &mut Ui, editor: &EditorHandle) {
    ui.input.dispatch_to(
        &ui.scene,
        editor.node,
        InputEvent::PointerDown {
            x: 5.,
            y: 1010.,
            button: PointerButton::Primary,
        },
    );
}
#[test]
fn line_insets_a_press_runs_are_answered() {
    let asker = Asker::new(200.);
    let (mut ui, editor) = in_room(&asker);
    asker.ask.set(true);
    press(&mut ui, &editor);
    assert_eq!(*asker.got.borrow(), [Some(1200.)]);
    assert!(editor.selection().1 > 300, "{:?}", editor.selection());
    *asker.handle.borrow_mut() = None;
}
/// A drag held below the area scrolls, selecting lines before they draw.
#[test]
fn line_insets_a_drag_runs_are_answered() {
    let asker = Asker::new(200.);
    let (mut ui, editor) = in_room(&asker);
    ui.dispatch(InputEvent::PointerDown {
        x: 5.,
        y: 10.,
        button: PointerButton::Primary,
    });
    ui.dispatch(InputEvent::PointerMove { x: 5., y: 390. });
    asker.ask.set(true);
    let next = ui.next_interaction_deadline().expect("it scrolls");
    ui.advance_interactions(next).unwrap();
    assert_eq!(*asker.got.borrow(), [Some(1200.)]);
    assert!(editor.selection().1 > 0);
    *asker.handle.borrow_mut() = None;
}
/// Atoms followed: what gives them runs with the model borrowed.
#[test]
fn atoms_when_is_answered() {
    let asker = Asker::new(200.);
    let asked = asker.clone();
    let mut version = None;
    let (_ui, _editor) = note(&asker, |ui, area| {
        let signal = ui.signal(0);
        version = Some(signal.clone());
        area.atoms_when(move |_| {
            signal.get();
            asked.run();
            Vec::new()
        })
    });
    asker.ask.set(true);
    version.unwrap().set(1);
    assert_eq!(*asker.got.borrow(), [Some(1200.)]);
    *asker.handle.borrow_mut() = None;
}
