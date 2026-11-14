//! A text area tells the width its text wraps in (`EditorHandle::wrap_width`, `on_wrap_width`): a picture's
//! frame in a line's room is as wide as that allows, so its height follows the width. Told once as the width
//! changes, after layout and before the frame is drawn: the room set again there is in that frame.
use std::{cell::RefCell, rc::Rc};
use zgx::{
    compose::prelude::*,
    scene::{NodeId, NodeKind, Rect},
    widgets::{EditorHandle, LineInset, Ui},
};

const TEXT: &str = "picture caption\nsecond\nthird";

fn area(ui: &mut Ui, text: &str) -> EditorHandle {
    let value = ui.signal(text.to_string());
    let view = ui.mount(
        text_area("Notes", value)
            .w_full()
            .h(300.)
            .p(0.)
            .text_size(10.)
            .line_height(20.)
            .text_wrap(true),
    );
    ui.prepare_frame();
    ui.editor(view.node()).unwrap()
}
fn line(ui: &Ui, editor: &EditorHandle, starts: &str) -> Rect {
    let scene = ui.scene.borrow();
    let mut pending: Vec<NodeId> = vec![editor.node];
    while let Some(id) = pending.pop() {
        if let NodeKind::Text { text, .. } = scene.kind(id)
            && text.starts_with(starts)
        {
            return scene.bounds(id);
        }
        pending.extend(scene.children(id).iter().copied());
    }
    panic!("a node for {starts}")
}

#[test]
fn a_text_area_knows_the_width_it_wraps_in() {
    let mut ui = Ui::new(500., 400.);
    let editor = area(&mut ui, TEXT);
    assert_eq!(editor.wrap_width(), Some(500.));
    // Not wrapped: none.
    let value = ui.signal("one line".to_string());
    let view = ui.mount(text_area("Plain", value).w(200.).h(40.).p(0.));
    ui.prepare_frame();
    let plain = ui.editor(view.node()).unwrap();
    assert_eq!(plain.wrap_width(), None);
}

#[test]
fn a_resize_tells_the_new_width_once() {
    let mut ui = Ui::new(500., 400.);
    let editor = area(&mut ui, TEXT);
    let told = Rc::new(RefCell::new(Vec::new()));
    let seen = told.clone();
    editor.on_wrap_width(move |width| seen.borrow_mut().push(width));
    ui.prepare_frame();
    assert!(told.borrow().is_empty(), "the width it has now isn't told");
    ui.scene.borrow_mut().resize(320., 400.);
    ui.prepare_frame();
    assert_eq!(*told.borrow(), vec![320.]);
    assert_eq!(editor.wrap_width(), Some(320.));
    ui.scene.borrow_mut().resize(410., 400.);
    ui.prepare_frame();
    ui.prepare_frame();
    assert_eq!(*told.borrow(), vec![320., 410.], "once for each width");
}

#[test]
fn room_set_as_the_width_is_told_is_in_that_frame() {
    let mut ui = Ui::new(500., 400.);
    let editor = area(&mut ui, TEXT);
    let first = line(&ui, &editor, "picture").y;
    // A picture of aspect 2:1 above the first line, as wide as the text wraps.
    let handle = editor.clone();
    editor.on_wrap_width(move |width| {
        handle.set_line_insets(Some(Rc::new(move |i| LineInset {
            above: if i == 0 { width / 2. } else { 0. },
            ..LineInset::default()
        })))
    });
    ui.scene.borrow_mut().resize(300., 400.);
    ui.prepare_frame();
    // One frame: the room for a 300-wide picture, 150 high, is already there.
    assert_eq!(line(&ui, &editor, "picture").y, first + 150.);
    ui.scene.borrow_mut().resize(400., 400.);
    ui.prepare_frame();
    assert_eq!(line(&ui, &editor, "picture").y, first + 200.);
}

#[test]
fn nothing_is_told_at_rest_or_as_it_scrolls() {
    let mut ui = Ui::new(500., 400.);
    let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let editor = area(&mut ui, &text);
    let told = Rc::new(RefCell::new(0_usize));
    let seen = told.clone();
    editor.on_wrap_width(move |_| *seen.borrow_mut() += 1);
    for _ in 0..5 {
        ui.prepare_frame();
    }
    editor.scroll_to_line(120, zgx::compose::RowAlign::Start);
    ui.prepare_frame();
    editor.scroll_to_line(10, zgx::compose::RowAlign::Start);
    ui.prepare_frame();
    assert_eq!(*told.borrow(), 0);
    // An idle window does no work: no frame draws the editor again (the
    // scroll's own paint flushed first).
    ui.scene.borrow_mut().flush();
    let draws = zgx::widgets::debug_editor_draws();
    for _ in 0..5 {
        ui.prepare_frame();
        assert!(ui.scene.borrow_mut().flush().is_idle());
    }
    assert_eq!(zgx::widgets::debug_editor_draws(), draws);
    assert_eq!(*told.borrow(), 0);
}
