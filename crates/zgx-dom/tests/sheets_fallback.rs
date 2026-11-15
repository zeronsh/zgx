//! zgx-dom's style sheet in a browser that can't adopt one: the rules go
//! in a <style> element instead, and the classes zgx writes still apply.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;
use zgx::{compose::column, reactive::Runtime, scene::Color, style::Styled};

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn without_adopted_sheets_a_style_element_holds_the_rules() {
    // A browser without `document.adoptedStyleSheets`.
    let global = js_sys::global();
    let document_class = js_sys::Reflect::get(&global, &JsValue::from_str("Document")).unwrap();
    let prototype = js_sys::Reflect::get(&document_class, &JsValue::from_str("prototype")).unwrap();
    let removed = js_sys::Reflect::delete_property(
        prototype.unchecked_ref::<js_sys::Object>(),
        &JsValue::from_str("adoptedStyleSheets"),
    )
    .unwrap();
    assert!(removed);
    let document = web_sys::window().unwrap().document().unwrap();
    assert!(
        !js_sys::Reflect::has(&document, &JsValue::from_str("adoptedStyleSheets")).unwrap(),
        "the page can't adopt sheets"
    );

    let runtime = Runtime::new();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let _mounted = zgx_dom::mount(&runtime, &host, column().w(40.).bg(Color(255, 0, 0, 255)));
    let styled = host
        .first_element_child()
        .unwrap()
        .first_element_child()
        .unwrap();
    let style = web_sys::window()
        .unwrap()
        .get_computed_style(&styled)
        .unwrap()
        .unwrap();
    assert_eq!(style.get_property_value("width").unwrap(), "40px");
    assert_eq!(
        style.get_property_value("background-color").unwrap(),
        "rgb(255, 0, 0)"
    );
    assert!(
        document.query_selector("head > style").unwrap().is_some(),
        "the rules are in a <style> element"
    );
}
