//! Luau apps (zgx-script) mounted as DOM, in a headless browser: Luau's VM
//! built for wasm32 with wasi-sdk (`luau-env.sh`), its script compiled and
//! run there, its state bound to the nodes that show it.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use zgx::reactive::Runtime;
use zgx_script::ScriptHost;

wasm_bindgen_test_configure!(run_in_browser);

fn host() -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    host
}

#[wasm_bindgen_test]
fn a_luau_app_runs_in_the_dom() {
    let runtime = Runtime::new();
    let script = ScriptHost::inline(
        &runtime,
        "app.luau",
        r#"
        local count = ui.state("count", 0)
        return ui.column {
            gap = 8,
            ui.button { on_click = ui.add(count, 1), "Add" },
            ui.text { "Clicked {count} times" },
        }
        "#,
    );
    script.reload().expect("the script loads");
    let host = host();
    let _mounted = zgx_dom::mount(&runtime, &host, script.view());
    let text = || host.text_content().unwrap_or_default();
    assert!(text().contains("Clicked 0 times"), "{}", text());
    let button: web_sys::HtmlElement = host
        .query_selector("button")
        .unwrap()
        .expect("a button")
        .dyn_into()
        .unwrap();
    button.click();
    button.click();
    zgx_dom::flush();
    assert!(text().contains("Clicked 2 times"), "{}", text());
}

#[wasm_bindgen_test]
fn script_errors_are_caught_in_the_browser() {
    // Luau throws on a script error (a C++ exception, through WebAssembly's
    // exception handling): the host reports it rather than the page dying.
    let runtime = Runtime::new();
    let script = ScriptHost::inline(&runtime, "broken.luau", "error('boom')");
    let error = script.reload().expect_err("the script fails");
    assert!(format!("{error:?}").contains("boom"), "{error:?}");
}
