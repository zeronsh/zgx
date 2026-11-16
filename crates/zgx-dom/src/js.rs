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
    pub(crate) fn zgx_tasks_init(
        run: &Closure<dyn FnMut()>,
        fire: &Closure<dyn FnMut()>,
        frame: &Closure<dyn FnMut(f64)>,
    );
    pub(crate) fn zgx_tasks_wake(yielded: bool);
    pub(crate) fn zgx_timer_arm(ms: f64);
    pub(crate) fn zgx_timer_armed() -> bool;
    pub(crate) fn zgx_frame_ask();
    pub(crate) fn zgx_trim_arm(fire: &Closure<dyn FnMut()>, ms: f64);
    pub(crate) fn zgx_frame_asked() -> bool;
    pub(crate) fn zgx_adopt(id: u32, element: &web_sys::Element);
    pub(crate) fn zgx_apply(memory: &JsValue, ops: u32, ops_len: u32, text: u32, text_len: u32);
    pub(crate) fn zgx_live_nodes() -> u32;
    pub(crate) fn zgx_drag_scroll_pending() -> bool;
    pub(crate) fn zgx_clipboard_text() -> Option<String>;
    pub(crate) fn zgx_clipboard_html() -> Option<String>;
    pub(crate) fn zgx_clipboard_write(text: &str, html: Option<String>) -> bool;
    pub(crate) fn zgx_open_url(url: &str) -> bool;
    pub(crate) fn zgx_scroll_into_view(id: u32, block: &str, inline: &str, animate: i32) -> bool;
    pub(crate) fn zgx_measure(font: &str, text: &str) -> Box<[f64]>;
    pub(crate) fn zgx_focus_snapshot() -> Option<Box<[f64]>>;
    pub(crate) fn zgx_restore_focus(id: u32, tag: u32, anchor: f64, focus: f64) -> bool;
}
