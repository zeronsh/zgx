//! Variable and measured virtual lists in the DOM: rows at their own
//! heights, measured as they lay out, the view kept on its row as heights
//! above it change.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::*;
use zgx::{
    compose::{VariableHeights, column, measured_virtual_list, text, variable_virtual_list},
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

/// The list: the mounted view's own element.
fn list(host: &web_sys::Element) -> web_sys::Element {
    host.first_element_child()
        .unwrap()
        .first_element_child()
        .unwrap()
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// Each row shown: its text, top and height in the list's space.
fn shown(list: &web_sys::Element) -> Vec<(String, f64, f64)> {
    let space = list.first_element_child().unwrap();
    let origin = space.get_bounding_client_rect().top();
    let rows = space.children();
    let mut out: Vec<_> = (0..rows.length())
        .map(|i| {
            let row = rows.item(i).unwrap();
            let rect = row.get_bounding_client_rect();
            (
                row.text_content().unwrap(),
                (rect.top() - origin).round(),
                rect.height().round(),
            )
        })
        .collect();
    out.sort_by(|a, b| a.1.total_cmp(&b.1));
    out
}

#[wasm_bindgen_test]
async fn variable_rows_lie_at_their_own_heights() {
    let runtime = Runtime::new();
    let heights = VariableHeights::new(&runtime, 1_000, 20.);
    heights.set_heights(
        &(0..1_000)
            .map(|i| (i, 20. + (i % 3) as f32 * 10.))
            .collect::<Vec<_>>(),
    );
    let offset = runtime.signal(0_f32);
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(
            variable_virtual_list(
                offset.clone(),
                heights.clone(),
                0,
                |i| i,
                |_, key: usize, _| text(format!("row {key}")),
            )
            .h(100.),
        ),
    );
    sleep(30).await;
    let list = list(&host).first_element_child().unwrap();
    let space = list.first_element_child().unwrap();
    // All rows: 20, 30, 40 repeating (90 a three), 333 threes and a 20.
    assert_eq!(
        space.get_attribute("style").unwrap(),
        "height: 29990px; flex-shrink: 0;"
    );
    let rows = shown(&list);
    assert_eq!(rows[0], ("row 0".into(), 0., 20.));
    assert_eq!(rows[1], ("row 1".into(), 20., 30.));
    assert_eq!(rows[2], ("row 2".into(), 50., 40.));
    assert_eq!(rows[3], ("row 3".into(), 90., 20.));
    assert_eq!(rows.len(), 4, "what shows in 100 points: {rows:?}");
    // Scrolled to row 300's top (100 threes in, 9,000 down).
    offset.set(9_000.);
    zgx_dom::flush();
    let rows = shown(&list);
    assert_eq!(rows[0], ("row 300".into(), 9_000., 20.));
    // A height above the view grows (row 10: 30 to 60): the view stays
    // on its row.
    heights.set_height(10, 60.).unwrap();
    zgx_dom::flush();
    assert_eq!(offset.get(), 9_030.);
    assert_eq!(shown(&list)[0], ("row 300".into(), 9_030., 20.));
    mounted.unmount();
    host.remove();
}

#[wasm_bindgen_test]
async fn measured_rows_take_the_height_they_lay_out_at() {
    let runtime = Runtime::new();
    let heights = VariableHeights::new(&runtime, 200, 50.);
    let offset = runtime.signal(0_f32);
    let read = heights.clone();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(
            measured_virtual_list(
                offset.clone(),
                heights.clone(),
                1,
                |i| i,
                |_, key: usize, _| {
                    column()
                        .h(10. + (key % 4) as f32 * 10.)
                        .child(text(format!("row {key}")))
                },
            )
            .h(120.),
        ),
    );
    // The rows lay out, are observed, and move to their heights.
    sleep(100).await;
    zgx_dom::flush();
    sleep(50).await;
    assert_eq!(read.row_height(0), Some(10.));
    assert_eq!(read.row_height(1), Some(20.));
    assert_eq!(read.row_height(2), Some(30.));
    assert_eq!(read.row_height(3), Some(40.));
    let list = list(&host).first_element_child().unwrap();
    let rows = shown(&list);
    assert_eq!(rows[0], ("row 0".into(), 0., 10.));
    assert_eq!(rows[1], ("row 1".into(), 10., 20.));
    assert_eq!(rows[2], ("row 2".into(), 30., 30.));
    assert_eq!(rows[3], ("row 3".into(), 60., 40.));
    // Rows not mounted keep the estimate.
    assert_eq!(read.row_height(150), Some(50.));
    mounted.unmount();
    host.remove();
}
