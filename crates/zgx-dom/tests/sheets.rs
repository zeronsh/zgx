//! zgx-dom's style sheet under a strict `style-src`: the page's CSP blocks
//! a <style> element, and the classes zgx writes still apply, from a sheet
//! the document adopts.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;
use zgx::{compose::column, reactive::Runtime, scene::Color, style::Styled};

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// The mounted view's own element (inside the root that carries the
/// theme's typography).
fn first(host: &web_sys::Element) -> web_sys::Element {
    host.first_element_child()
        .unwrap()
        .first_element_child()
        .unwrap()
}

fn width_and_background(element: &web_sys::Element) -> (String, String) {
    let style = web_sys::window()
        .unwrap()
        .get_computed_style(element)
        .unwrap()
        .unwrap();
    (
        style.get_property_value("width").unwrap(),
        style.get_property_value("background-color").unwrap(),
    )
}

#[wasm_bindgen_test]
fn classes_apply_under_a_style_src_that_blocks_style_elements() {
    let document = document();
    let head = document.query_selector("head").unwrap().unwrap();
    let csp = document.create_element("meta").unwrap();
    csp.set_attribute("http-equiv", "Content-Security-Policy")
        .unwrap();
    csp.set_attribute("content", "style-src 'self'").unwrap();
    head.append_child(&csp).unwrap();

    // The policy holds: a <style> element's rules don't apply.
    let probe = document.create_element("div").unwrap();
    probe.set_class_name("zgx-csp-probe");
    document.body().unwrap().append_child(&probe).unwrap();
    let blocked = document.create_element("style").unwrap();
    blocked.set_text_content(Some(".zgx-csp-probe { width: 7px }"));
    head.append_child(&blocked).unwrap();
    assert_ne!(
        width_and_background(&probe).0,
        "7px",
        "the CSP blocks <style>"
    );
    blocked.remove();

    let runtime = Runtime::new();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let _mounted = zgx_dom::mount(&runtime, &host, column().w(40.).bg(Color(255, 0, 0, 255)));
    let styled = first(&host);
    assert_eq!(width_and_background(&styled).0, "40px");
    assert_eq!(width_and_background(&styled).1, "rgb(255, 0, 0)");

    let adopted =
        js_sys::Reflect::get(&document, &JsValue::from_str("adoptedStyleSheets")).unwrap();
    assert!(
        js_sys::Array::from(&adopted).length() >= 1,
        "zgx's sheet is adopted"
    );
    assert!(
        document.query_selector("style").unwrap().is_none(),
        "no <style> element is added"
    );
}
