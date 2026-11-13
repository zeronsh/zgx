//! The JS side's functions (`js/zgx-dom.js`).

use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/js/zgx-dom.js")]
extern "C" {
    pub(crate) fn zgx_init(
        base_css: &str,
        dispatch: &Closure<dyn FnMut(u32, u32, f64, Option<String>)>,
        flush: &Closure<dyn FnMut()>,
        event: &Closure<crate::mount::EventFn>,
        keys: &Closure<dyn FnMut(String, String, String, u32) -> u32>,
    );
    pub(crate) fn zgx_schedule();
    pub(crate) fn zgx_adopt(id: u32, element: &web_sys::Element);
    pub(crate) fn zgx_apply(memory: &JsValue, ops: u32, ops_len: u32, text: u32, text_len: u32);
    pub(crate) fn zgx_live_nodes() -> u32;
}
