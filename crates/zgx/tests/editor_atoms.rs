use std::{cell::RefCell, rc::Rc};
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers},
    scene::Color,
    text_edit::{Atom, AtomChange},
    widgets::{EditorHandle, Ui},
};

fn field() -> (Ui, EditorHandle) {
    let mut ui = Ui::new(400., 200.);
    let value = ui.signal(String::from("see  now"));
    let view = ui.mount(
        column()
            .size(400., 200.)
            .child(text_area("Notes", value).id("field").w(300.).h(80.)),
    );
    ui.prepare_frame();
    let node = view.find("field").unwrap();
    ui.input.focus(&ui.scene, Some(node));
    let editor = ui.editor(node).unwrap();
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

#[test]
fn an_atom_deletes_whole_and_the_listener_hears_it_go() {
    let (mut ui, editor) = field();
    let heard: Rc<RefCell<Vec<AtomChange>>> = Rc::default();
    let sink = heard.clone();
    editor.on_atoms(move |event| sink.borrow_mut().extend_from_slice(event.changes));
    editor.set_selection(4, 4);
    let id = editor
        .insert_atom("@file.pdf", None, None, None)
        .expect("an atom");
    ui.prepare_frame();
    assert_eq!(editor.editor.borrow().text(), "see @file.pdf now");
    assert_eq!(editor.atoms(), [Atom::new(id, 4..13)]);
    assert!(
        heard
            .borrow()
            .contains(&AtomChange::Added(Atom::new(id, 4..13)))
    );
    key(&mut ui, Key::Backspace);
    assert_eq!(editor.editor.borrow().text(), "see  now");
    assert!(editor.atoms().is_empty());
    assert!(
        heard
            .borrow()
            .contains(&AtomChange::Removed(Atom::new(id, 4..13), 4..13, "@file.pdf".into())),
        "{:?}",
        heard.borrow()
    );
    assert!(editor.undo());
    assert_eq!(editor.atoms(), [Atom::new(id, 4..13)]);
}

#[test]
fn an_atom_with_a_wash_adds_a_layer_that_draws_it() {
    let nodes = |wash: bool| {
        let (ui, editor) = field();
        if wash {
            editor.set_atom_wash(Some(Color(0, 0, 255, 60)));
        }
        editor.set_selection(4, 4);
        editor.insert_atom("@file.pdf", None, None, None).unwrap();
        ui.prepare_frame();
        ui.scene.borrow().len()
    };
    assert!(
        nodes(true) > nodes(false),
        "the wash draws as a mark of its own"
    );
}
