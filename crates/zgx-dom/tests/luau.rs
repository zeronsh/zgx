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

/// Wait `ms` of the browser's time, its tasks running meanwhile.
async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

#[wasm_bindgen_test]
async fn luau_timers_and_promises_run_in_the_browser() {
    // `ui.after`, `ui.every` and a promise's future run on the view's
    // tasks: inert in the browser until the mount gave it a runner.
    let runtime = Runtime::new();
    let script = ScriptHost::inline(
        &runtime,
        "async.luau",
        r#"
        local after = ui.state("after", 0)
        local ticks = ui.state("ticks", 0)
        local got = ui.state("got", "none")
        ui.after(20, function()
            after:set(1)
        end)
        ui.every(10, function()
            ticks:set(ticks:get() + 1)
        end)
        ui.task(function()
            local doubled = ui.call("double", 21):await()
            got:set(`got {doubled}`)
        end)
        return ui.column {
            ui.text { "after {after}" },
            ui.text { "ticks {ticks}" },
            ui.text { "{got}" },
        }
        "#,
    );
    script.async_action("double", |arg| async move {
        zgx::timer::sleep(std::time::Duration::from_millis(5)).await;
        match arg {
            zgx_script::Value::Num(n) => Ok(zgx_script::Value::Num(n * 2.)),
            _ => Err(String::from("a number")),
        }
    });
    script.reload().expect("the script loads");
    let host = host();
    let mounted = zgx_dom::mount(&runtime, &host, script.view());
    let text = || host.text_content().unwrap_or_default();
    assert!(text().contains("after 0"), "{}", text());
    sleep(100).await;
    assert!(text().contains("after 1"), "ui.after fired: {}", text());
    assert!(
        text().contains("got 42"),
        "the promise resolved: {}",
        text()
    );
    let ticks: u32 = text()
        .split("ticks ")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|n| n.parse().ok())
        .expect("a tick count");
    assert!(ticks >= 3, "ui.every ticked: {}", text());
    assert!(zgx_dom::timer_armed(), "ui.every's next tick is armed");
    // Unmounted, its tasks go: `ui.every` stops, and nothing is armed.
    mounted.unmount();
    sleep(40).await;
    assert!(!zgx_dom::timer_armed(), "nothing waits once it's gone");
}

#[wasm_bindgen_test]
async fn luau_tweens_springs_and_anime_run_in_the_browser() {
    // `ui.tween`, `ui.spring_to` and `anime.animate` tick on the page's
    // frame clock: none of them moved in the browser without one.
    let runtime = Runtime::new();
    let script = ScriptHost::inline(
        &runtime,
        "anime.luau",
        r#"
        local to = ui.state("to", 10)
        local at = ui.state("at", 0)
        local n = ui.state("n", 0)
        local said = ui.state("said", "")
        return ui.column {
            ui.div { id = "box", h = 10, w = ui.tween(to, { duration = 80 }) },
            ui.text { "at {at}" },
            ui.text { "n {n}" },
            ui.text { "said {said}" },
            ui.button { "Go", on_click = function()
                to:set(120)
                ui.spring_to(at, 100, { stiffness = 600, damping = 48.99, done = function()
                    said:set(said:get() .. "spring ")
                end })
                anime.animate(n, {
                    value = 50,
                    duration = 80,
                    ease = "linear",
                    modifier = anime.utils.round(0),
                    onComplete = function()
                        said:set(said:get() .. "anime ")
                    end,
                })
            end },
        }
        "#,
    );
    script.reload().expect("the script loads");
    let host = host();
    let mounted = zgx_dom::mount(&runtime, &host, script.view());
    sleep(50).await;
    assert!(!zgx_dom::frame_asked(), "at rest: no frame asked");
    let button: web_sys::HtmlElement = host
        .query_selector("button")
        .unwrap()
        .expect("a button")
        .dyn_into()
        .unwrap();
    button.click();
    sleep(800).await;
    zgx_dom::flush();
    let text = host.text_content().unwrap_or_default();
    assert!(text.contains("spring "), "the spring settled: {text}");
    assert!(text.contains("anime "), "anime.animate finished: {text}");
    assert!(text.contains("n 50"), "{text}");
    assert!(text.contains("at 100"), "{text}");
    let width = host
        .query_selector("[data-id=box]")
        .unwrap()
        .map(|el| el.get_bounding_client_rect().width());
    assert_eq!(width, Some(120.), "the tween reached its target");
    sleep(50).await;
    assert!(!zgx_dom::frame_asked(), "all settled: no frame asked");
    assert!(!zgx_dom::timer_armed());
    mounted.unmount();
}
