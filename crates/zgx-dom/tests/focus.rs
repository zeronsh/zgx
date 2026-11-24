//! `NodeHandle::focus_snapshot` and `restore_focus` on the DOM, in a
//! headless browser: the document's focus and a field's selection, in UTF-8
//! bytes of its value, put back with nothing scrolled.
#![cfg(target_arch = "wasm32")]

use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use zgx::{
    compose::{NodeHandle, button, column, scroll, text, text_input},
    reactive::Runtime,
    style::Styled,
};

wasm_bindgen_test_configure!(run_in_browser);

fn host() -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    host
}

fn active() -> Option<web_sys::Element> {
    web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .active_element()
}

fn field(host: &web_sys::Element) -> web_sys::HtmlInputElement {
    host.query_selector("input")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap()
}

fn the_button(host: &web_sys::Element) -> web_sys::HtmlElement {
    host.query_selector("button")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap()
}

#[wasm_bindgen_test]
fn focus_and_a_fields_selection_come_back() {
    let runtime = Runtime::new();
    let value = runtime.signal(String::from("héllo wörld"));
    let handle: Rc<RefCell<Option<NodeHandle>>> = Rc::default();
    let keep = handle.clone();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column()
            .child(text_input("Name", value.clone()))
            .child(button().child(text("Elsewhere")))
            // A page taller than the window, scrolled: restoring scrolls none.
            .child(column().h(4000.))
            .on_mount(move |h| *keep.borrow_mut() = Some(h)),
    );
    let handle = handle.borrow().clone().expect("a handle");
    let input = field(&host);
    input.focus().unwrap();
    // "hél" backwards: UTF-16 1..3 are UTF-8 bytes 1..4 ("é" is two).
    input
        .set_selection_range_with_direction(1, 3, "backward")
        .unwrap();
    let snapshot = handle.focus_snapshot().expect("focus on the field");
    assert_eq!(snapshot.selection, Some((4, 1)), "anchor at its end");

    the_button(&host).focus().unwrap();
    input.set_selection_range(0, 0).unwrap();
    let window = web_sys::window().unwrap();
    window.scroll_to_with_x_and_y(0., 2000.);
    let scrolled = window.scroll_y().unwrap();
    assert!(handle.restore_focus(&snapshot));
    assert_eq!(
        active().as_ref(),
        Some(input.unchecked_ref::<web_sys::Element>())
    );
    assert_eq!(input.selection_start().unwrap(), Some(1));
    assert_eq!(input.selection_end().unwrap(), Some(3));
    assert_eq!(
        input.selection_direction().unwrap().as_deref(),
        Some("backward")
    );
    assert_eq!(window.scroll_y().unwrap(), scrolled, "nothing scrolled");
    window.scroll_to_with_x_and_y(0., 0.);

    // A button has focus and no selection.
    the_button(&host).focus().unwrap();
    let on_button = handle.focus_snapshot().expect("focus on the button");
    assert_eq!(on_button.selection, None);
    input.focus().unwrap();
    assert!(handle.restore_focus(&on_button));
    assert_eq!(
        active().as_ref(),
        Some(the_button(&host).unchecked_ref::<web_sys::Element>())
    );

    // Offsets the text no longer holds: inside "é", or past a shorter end.
    // False, and focus stays where it is.
    let inside = zgx::widgets::FocusSnapshot {
        selection: Some((2, 2)),
        ..snapshot
    };
    assert!(!handle.restore_focus(&inside), "byte 2 is inside é");
    value.set(String::from("h"));
    zgx_dom::flush();
    assert!(!handle.restore_focus(&snapshot), "past the end");
    assert_eq!(
        active().as_ref(),
        Some(the_button(&host).unchecked_ref::<web_sys::Element>())
    );
    mounted.unmount();
}

#[wasm_bindgen_test]
fn a_gone_node_is_not_restored() {
    let runtime = Runtime::new();
    let handle: Rc<RefCell<Option<NodeHandle>>> = Rc::default();
    let keep = handle.clone();
    let host = host();
    let outer = zgx_dom::mount(
        &runtime,
        &host,
        button()
            .child(text("Stays"))
            .on_mount(move |h| *keep.borrow_mut() = Some(h)),
    );
    let handle = handle.borrow().clone().expect("a handle");
    let other = self::host();
    let inner = zgx_dom::mount(
        &runtime,
        &other,
        text_input("Goes", runtime.signal(String::new())),
    );
    field(&other).focus().unwrap();
    let snapshot = handle.focus_snapshot().expect("focus on the field");
    the_button(&host).focus().unwrap();
    inner.unmount();
    // Its slot made again, for another node: still not it.
    let again = zgx_dom::mount(
        &runtime,
        &other,
        text_input("New", runtime.signal(String::new())),
    );
    assert!(!handle.restore_focus(&snapshot));
    assert_eq!(
        active().as_ref(),
        Some(the_button(&host).unchecked_ref::<web_sys::Element>())
    );
    // Focus on nothing of ours: no snapshot.
    the_button(&host).blur().unwrap();
    assert!(handle.focus_snapshot().is_none());
    again.unmount();
    outer.unmount();
}

#[wasm_bindgen_test]
fn an_older_snapshot_of_a_reused_slot_is_not_restored() {
    // A snapshot of a node, then one of another made in the slot it freed:
    // the first still names the node that went.
    let runtime = Runtime::new();
    let handle: Rc<RefCell<Option<NodeHandle>>> = Rc::default();
    let keep = handle.clone();
    let host = host();
    let outer = zgx_dom::mount(
        &runtime,
        &host,
        button()
            .child(text("Stays"))
            .on_mount(move |h| *keep.borrow_mut() = Some(h)),
    );
    let handle = handle.borrow().clone().expect("a handle");
    let other = self::host();
    let first = zgx_dom::mount(
        &runtime,
        &other,
        text_input("First", runtime.signal(String::new())),
    );
    field(&other).focus().unwrap();
    let older = handle.focus_snapshot().expect("focus on the first field");
    first.unmount();
    let second = zgx_dom::mount(
        &runtime,
        &other,
        text_input("Second", runtime.signal(String::new())),
    );
    field(&other).focus().unwrap();
    let newer = handle.focus_snapshot().expect("focus on the second field");
    assert_eq!(
        older.node.number(),
        newer.node.number(),
        "the slot was reused"
    );
    the_button(&host).focus().unwrap();
    assert!(!handle.restore_focus(&older), "the first field went");
    assert_eq!(
        active().as_ref(),
        Some(the_button(&host).unchecked_ref::<web_sys::Element>())
    );
    assert!(handle.restore_focus(&newer));
    second.unmount();
    outer.unmount();
}

#[wasm_bindgen_test]
fn a_scroll_view_around_the_field_stays_where_it_is() {
    let runtime = Runtime::new();
    let offset = runtime.signal(0_f32);
    let handle: Rc<RefCell<Option<NodeHandle>>> = Rc::default();
    let keep = handle.clone();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column()
            .child(button().child(text("Elsewhere")))
            .child(
                scroll(offset)
                    .h(100.)
                    .child(column().h(600.))
                    .child(text_input("Far", runtime.signal(String::from("far down")))),
            )
            .on_mount(move |h| *keep.borrow_mut() = Some(h)),
    );
    let handle = handle.borrow().clone().expect("a handle");
    let input = field(&host);
    // Focusing it scrolls it into view; the snapshot is taken there.
    input.focus().unwrap();
    input.set_selection_range(4, 8).unwrap();
    let snapshot = handle.focus_snapshot().expect("focus on the field");
    let viewport = input
        .parent_element()
        .and_then(|el| {
            let mut el = Some(el);
            while let Some(e) = el {
                if e.scroll_height() > e.client_height() + 1 {
                    return Some(e);
                }
                el = e.parent_element();
            }
            None
        })
        .expect("the scroll view around it");
    the_button(&host).focus().unwrap();
    viewport.set_scroll_top(0);
    assert!(handle.restore_focus(&snapshot));
    assert_eq!(
        active().as_ref(),
        Some(input.unchecked_ref::<web_sys::Element>())
    );
    assert_eq!(input.selection_start().unwrap(), Some(4));
    assert_eq!(viewport.scroll_top(), 0, "the scroll view didn't move");
    mounted.unmount();
}
