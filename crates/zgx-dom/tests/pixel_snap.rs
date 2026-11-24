//! A view that snaps (`View::pixel_snap`) rests on whole device pixels in
//! the DOM too: its translation, static or reactive, is rounded by the page
//! to `--zgx-device-px`, and a view that doesn't snap keeps its exact offset.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::*;
use zgx::{
    compose::{column, div},
    reactive::Runtime,
    style::{Styled, Styles},
};

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}
/// The element with view id `id`: across, as its computed transform moves it.
fn moved_x(host: &web_sys::Element, id: &str) -> f64 {
    let element = host
        .query_selector(&format!("[data-id=\"{id}\"]"))
        .unwrap()
        .unwrap_or_else(|| panic!("no {id}"));
    let transform = web_sys::window()
        .unwrap()
        .get_computed_style(&element)
        .unwrap()
        .unwrap()
        .get_property_value("transform")
        .unwrap();
    // matrix(a, b, c, d, x, y)
    let inner = transform
        .trim_start_matches("matrix(")
        .trim_end_matches(')');
    inner
        .split(',')
        .nth(4)
        .map_or(0., |x| x.trim().parse().unwrap())
}

#[wasm_bindgen_test]
fn a_snapping_views_translation_rests_on_a_device_pixel() {
    let document = document();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let runtime = Runtime::new();
    let x = runtime.signal(7.3_f32);
    let read = x.clone();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column()
            .child(
                div()
                    .id("snap")
                    .size(20., 20.)
                    .translate(10.3, 0.)
                    .pixel_snap(),
            )
            .child(div().id("exact").size(20., 20.).translate(10.3, 0.))
            .child(
                div()
                    .id("moving")
                    .size(20., 20.)
                    .reactive_style(move || Styles::new().translate(read.get(), 0.))
                    .pixel_snap(),
            ),
    );
    // A 2x screen: one device pixel is half a CSS pixel.
    let root = document.document_element().unwrap();
    let style = wasm_bindgen::JsCast::unchecked_ref::<web_sys::HtmlElement>(&root).style();
    style.set_property("--zgx-device-px", "0.5px").unwrap();
    assert_eq!(moved_x(&host, "snap"), 10.5);
    assert!(
        (moved_x(&host, "exact") - 10.3).abs() < 1e-3,
        "a view that doesn't snap is exact"
    );
    assert_eq!(moved_x(&host, "moving"), 7.5);
    x.set(7.2);
    zgx_dom::flush();
    assert_eq!(moved_x(&host, "moving"), 7.);
    // A 1x screen: whole CSS pixels.
    style.set_property("--zgx-device-px", "1px").unwrap();
    assert_eq!(moved_x(&host, "snap"), 10.);
    mounted.unmount();
    host.remove();
}
