//! A disabled element and what it holds, as a disabled fieldset's controls
//! are: no clicks or focus reach them, their own disabled styles show, and
//! they are heard disabled. Enabled again, each is as it was, its own
//! disabled state kept.
#![cfg(target_arch = "wasm32")]

use std::{cell::Cell, rc::Rc};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use zgx::{
    compose::{button, column, div, text},
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
fn find(host: &web_sys::Element, selector: &str) -> web_sys::HtmlElement {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"))
        .dyn_into()
        .unwrap()
}
fn disabled(element: &web_sys::HtmlElement) -> bool {
    element
        .dyn_ref::<web_sys::HtmlButtonElement>()
        .is_some_and(|b| b.disabled())
}
fn opacity(element: &web_sys::HtmlElement) -> String {
    web_sys::window()
        .unwrap()
        .get_computed_style(element)
        .unwrap()
        .unwrap()
        .get_property_value("opacity")
        .unwrap()
}

#[wasm_bindgen_test]
fn a_disabled_element_disables_what_it_holds_and_gives_it_back() {
    let runtime = Runtime::new();
    let (off, locked) = (runtime.signal(true), runtime.signal(true));
    let (read_off, read_locked) = (off.clone(), locked.clone());
    let clicks = Rc::new(Cell::new(0));
    let count = clicks.clone();
    let host = host();
    let _mounted = zgx_dom::mount(
        &runtime,
        &host,
        column()
            .id("group")
            .disabled_when(move || read_off.get())
            .child(
                button()
                    .id("go")
                    .child(text("Go"))
                    .on_click(move || count.set(count.get() + 1)),
            )
            .child(
                button()
                    .id("locked")
                    .child(text("Locked"))
                    .disabled_when(move || read_locked.get()),
            )
            .child(
                div()
                    .id("styled")
                    .w(10.)
                    .h(10.)
                    .disabled_style(|s| s.opacity(0.6)),
            ),
    );
    let (group, go, locked_button, styled) = (
        find(&host, "[data-id=group]"),
        find(&host, "[data-id=go]"),
        find(&host, "[data-id=locked]"),
        find(&host, "[data-id=styled]"),
    );
    assert_eq!(
        group.get_attribute("aria-disabled").as_deref(),
        Some("true")
    );
    assert!(disabled(&go), "a button inside is disabled");
    go.click();
    assert_eq!(clicks.get(), 0, "no click reaches it");
    go.focus().unwrap();
    let active = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .active_element();
    assert!(
        !active.is_some_and(|a| a.is_same_node(Some(&go))),
        "nor focus"
    );
    assert_eq!(opacity(&styled), "0.6", "its own disabled style shows");

    off.set(false);
    zgx_dom::flush();
    assert_eq!(group.get_attribute("aria-disabled"), None);
    assert!(!disabled(&go), "enabled again");
    go.click();
    assert_eq!(clicks.get(), 1, "and it clicks");
    assert!(disabled(&locked_button), "one disabled itself stays so");
    assert_eq!(opacity(&styled), "1");

    // Its own state changed while the group held it: kept for after.
    off.set(true);
    zgx_dom::flush();
    locked.set(false);
    zgx_dom::flush();
    assert!(disabled(&locked_button), "the group still holds it");
    off.set(false);
    zgx_dom::flush();
    assert!(!disabled(&locked_button), "its own state, given back");
}
