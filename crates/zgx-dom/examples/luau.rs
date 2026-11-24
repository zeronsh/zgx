//! A Luau app in the browser: zgx-script runs the script below in Luau's VM
//! (built for wasm32 with wasi-sdk, `luau-env.sh`) and zgx-dom mounts what it
//! returns. Build it with `bench/build.sh luau`.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::prelude::*;
use zgx::reactive::Runtime;
use zgx_script::ScriptHost;

const APP: &str = r#"
local count = ui.state("count", 0)
local items = ui.state("items", {} :: { string })
return ui.column {
    p = 16,
    gap = 10,
    ui.row {
        gap = 8,
        ui.button { on_click = ui.add(count, 1), px = 10, py = 6, "Add" },
        ui.button {
            on_click = function()
                items:push("item " .. tostring(#items:get() + 1))
            end,
            px = 10,
            py = 6,
            "Push",
        },
    },
    ui.text { "Clicked {count} times" },
    ui.each(items, function(item: string) return ui.text { item } end),
}
"#;

#[wasm_bindgen(start)]
pub fn start() {
    let started = web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now());
    let runtime = Runtime::new();
    let script = ScriptHost::inline(&runtime, "app.luau", APP);
    if let Err(error) = script.reload() {
        web_sys::console::error_1(&error.into());
    }
    std::mem::forget(zgx_dom::mount_in_body(&runtime, script.view()));
    if let (Some(started), Some(now)) = (
        started,
        web_sys::window()
            .and_then(|w| w.performance())
            .map(|p| p.now()),
    ) {
        // For the benchmark: how long the VM, the script and the mount took.
        let _ = js_sys::Reflect::set(
            &js_sys::global(),
            &"zgxMountMs".into(),
            &(now - started).into(),
        );
    }
    std::mem::forget(script);
}
