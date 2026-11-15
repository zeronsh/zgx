//! A panic the page caught: where panics abort (wasm32), a handler's panic
//! traps, the page's JS carries on, and `std::thread::panicking` stays
//! true. Effects made after it still run.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_test::*;
use zgx::{
    compose::{column, text, text_signal},
    reactive::Runtime,
};

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn effects_run_after_a_panic_the_page_caught() {
    // A handler that panics, called from JS that catches the trap.
    let handler = Closure::<dyn FnMut()>::new(|| panic!("a handler's panic"));
    let call = js_sys::Function::new_with_args(
        "f",
        "try { f(); return false; } catch (_) { return true; }",
    );
    let caught = call
        .call1(&JsValue::NULL, handler.as_ref().unchecked_ref())
        .unwrap();
    assert_eq!(caught, JsValue::TRUE, "the panic trapped and was caught");
    assert!(std::thread::panicking(), "a caught trap leaves this set");

    let runtime = Runtime::new();
    let count = runtime.signal(0);
    let shown = count.clone();
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column()
            .child(text("Count "))
            .child(text_signal(move || format!("{}", shown.get()))),
    );
    assert_eq!(
        host.text_content().unwrap(),
        "Count 0",
        "a mount's effects ran"
    );
    count.set(3);
    zgx_dom::flush();
    assert_eq!(host.text_content().unwrap(), "Count 3", "and run again");
    runtime.batch(|| count.set(4));
    zgx_dom::flush();
    assert_eq!(host.text_content().unwrap(), "Count 4", "a batch flushes");
    mounted.unmount();
    host.remove();
}
