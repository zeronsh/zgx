//! What `EditorHandle::text_metrics` answers where an app may ask it around
//! an edit, at the one width it always asks: an edit's text at once, a value
//! the app sets once the area has taken it.
use std::{cell::RefCell, rc::Rc};
use zgx::{
    compose::prelude::*,
    input::InputEvent,
    reactive::Signal,
    widgets::{EditorHandle, Ui},
};

type Log = Rc<RefCell<Vec<(String, f32)>>>;
fn ask(log: &Log, editor: &EditorHandle, place: &str) {
    let got = editor.text_metrics(200.).map_or(-1., |m| m.wrapped_height);
    log.borrow_mut().push((place.to_string(), got));
}
/// What `act` was answered in a note of two lines (40 points): in the value's
/// effect, made `first` (before the area) or after it, at once after, and in
/// the frame.
fn run(first: bool, act: &dyn Fn(&mut Ui, &EditorHandle, &Signal<String>)) -> Vec<(String, f32)> {
    let mut ui = Ui::new(600., 400.);
    let value = ui.signal(String::from("one\ntwo"));
    let kept: Rc<RefCell<Option<EditorHandle>>> = Rc::default();
    let log: Log = Rc::default();
    let effect = {
        let (kept, log, value) = (kept.clone(), log.clone(), value.clone());
        move || {
            value.get();
            if let Some(editor) = kept.borrow().as_ref() {
                ask(&log, editor, "effect");
            }
        }
    };
    let _early = first.then(|| ui.runtime.effect(effect.clone()));
    let view = ui.mount(
        text_area("Note", value.clone())
            .w(200.)
            .h(100.)
            .p(0.)
            .text_size(10.)
            .line_height(20.)
            .text_wrap(true),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    *kept.borrow_mut() = Some(editor.clone());
    if !first {
        ui.bind(view.node(), effect);
    }
    editor.set_selection(3, 3);
    ui.input.focus(&ui.scene, Some(view.node()));
    // Read by the effect, at rest.
    value.set("one\ntwo\nthree".into());
    ui.prepare_frame();
    value.set("one\ntwo".into());
    for _ in 0..3 {
        ui.prepare_frame();
    }
    editor.set_selection(3, 3);
    ui.prepare_frame();
    ask(&log, &editor, "before");
    assert_eq!(log.borrow().last().unwrap().1, 40.);
    log.borrow_mut().clear();
    act(&mut ui, &editor, &value);
    ask(&log, &editor, "at once");
    for _ in 0..3 {
        ui.prepare_frame();
    }
    *kept.borrow_mut() = None;
    log.take()
}
fn rows(effect: f32, then: f32) -> Vec<(String, f32)> {
    vec![
        ("effect".into(), effect),
        ("at once".into(), then),
        ("effect".into(), then),
    ]
}
#[test]
fn an_edit_is_measured_in_its_own_turn() {
    for first in [true, false] {
        let typed = run(first, &|ui, _, _| {
            ui.dispatch(InputEvent::Text("\n".into()));
        });
        assert_eq!(typed, rows(60., 60.), "typed, effect first {first}");
        let pasted = run(first, &|_, editor, _| editor.paste("a\nb\n"));
        assert_eq!(pasted, rows(80., 80.), "pasted, effect first {first}");
        let undone = run(first, &|ui, editor, _| {
            editor.paste("a\nb\n");
            ui.prepare_frame();
            assert!(editor.undo());
        });
        let undone = &undone[undone.len() - 3..];
        assert_eq!(undone, rows(40., 40.), "undone, effect first {first}");
    }
}
/// The effect made before the area runs before the area takes the value: it
/// is told the text before, and again once the frame is laid out.
#[test]
fn a_value_the_app_sets_is_measured_once_the_area_takes_it() {
    let set = |_: &mut Ui, _: &EditorHandle, value: &Signal<String>| {
        value.set("one\ntwo\nthree\nfour".into());
    };
    assert_eq!(run(true, &set), rows(40., 80.));
    assert_eq!(run(false, &set), rows(80., 80.));
}
