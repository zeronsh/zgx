//! A Luau app's `ui.fetch` through the page's `fetch` (`install_fetcher`),
//! in a headless browser and with no server: `data:` and `blob:` URLs for
//! bytes, a policy that refuses with the URL it was asked for to show how
//! a relative URL resolved, a name that doesn't resolve for a network
//! failure, and a cancel that aborts the request and leaves no error on
//! the page.
#![cfg(target_arch = "wasm32")]

use std::sync::{Arc, Mutex};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use zgx::reactive::Runtime;
use zgx_script::{FetchPolicy, ScriptHost, Url};

wasm_bindgen_test_configure!(run_in_browser);

/// A script showing what `ui.fetch(URL)` settled with: `status body`, or
/// `kind: message`.
fn app_source(url: &str) -> String {
    format!(
        r#"
        local p = ui.fetch("{url}")
        return ui.text {{
            id = "out",
            function()
                if not p.done then return "loading" end
                if p.error then return `{{p.error.kind}}: {{p.error.message}}` end
                return `{{p.value.status}} {{p.value.body}}`
            end,
        }}
        "#
    )
}

struct App {
    host: web_sys::Element,
    _mounted: zgx_dom::Mounted,
    _script: ScriptHost,
}

fn mount(runtime: &Runtime, script: ScriptHost) -> App {
    script.reload().expect("the script loads");
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let mounted = zgx_dom::mount(runtime, &host, script.view());
    App {
        host,
        _mounted: mounted,
        _script: script,
    }
}

async fn tick() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 5)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    zgx_dom::flush();
}

/// What the app shows once its fetch has settled.
async fn settled(app: &App) -> String {
    for _ in 0..400 {
        tick().await;
        let text = app.host.text_content().unwrap_or_default();
        if text != "loading" {
            return text;
        }
    }
    panic!(
        "the fetch never settled: the app shows {:?}, {} requests pending",
        app.host.text_content(),
        zgx_dom::__fetches_pending()
    );
}

fn fetching(url: &str) -> App {
    let runtime = Runtime::new();
    let script = ScriptHost::inline(&runtime, "app.luau", app_source(url));
    zgx_dom::install_fetcher(&script);
    mount(&runtime, script)
}

#[wasm_bindgen_test]
async fn a_data_url_comes_back_whole() {
    let app = fetching("data:text/plain,hello%20there");
    assert_eq!(settled(&app).await, "200 hello there");
}

#[wasm_bindgen_test]
async fn a_blob_url_comes_back_whole() {
    let make = js_sys::Function::new_no_args(
        "return URL.createObjectURL(new Blob([new Uint8Array([98, 121, 116, 101, 115])]))",
    );
    let url = make
        .call0(&wasm_bindgen::JsValue::NULL)
        .unwrap()
        .as_string()
        .unwrap();
    let app = fetching(&url);
    assert_eq!(settled(&app).await, "200 bytes");
}

/// Refuses every URL, saying which: how a relative one resolved.
struct Says(Arc<Mutex<Vec<String>>>);
impl FetchPolicy for Says {
    fn url(&self, _caller: Option<&str>, url: &Url) -> Result<(), String> {
        self.0.lock().unwrap().push(url.as_str().to_owned());
        Err(format!("asked for {}", url.as_str()))
    }
}

/// Takes the test's `<base>` away: the page's base is its own URL again.
fn no_base() {
    let document = web_sys::window().unwrap().document().unwrap();
    if let Some(base) = document.query_selector("base").unwrap() {
        base.remove();
    }
}

fn base(href: &str) {
    let document = web_sys::window().unwrap().document().unwrap();
    let base = match document.query_selector("base").unwrap() {
        Some(base) => base,
        None => {
            let base = document.create_element("base").unwrap();
            let head = document.query_selector("head").unwrap().unwrap();
            head.append_child(&base).unwrap();
            base
        }
    };
    base.set_attribute("href", href).unwrap();
}

#[wasm_bindgen_test]
async fn a_relative_url_resolves_against_the_base_it_was_installed_with() {
    base("https://first.invalid/app/");
    let asked = Arc::new(Mutex::new(Vec::new()));
    let runtime = Runtime::new();
    let script = ScriptHost::inline(&runtime, "app.luau", app_source("sounds/click.wav"));
    script.fetch_policy(Says(asked.clone()));
    zgx_dom::install_fetcher(&script);
    // The page's base moves after (back to the page's own URL), before
    // anything mounts: the app's doesn't.
    no_base();
    let app = mount(&runtime, script);
    for _ in 0..400 {
        if !asked.lock().unwrap().is_empty() {
            break;
        }
        tick().await;
    }
    assert_eq!(
        asked.lock().unwrap().first().map(String::as_str),
        Some("https://first.invalid/app/sounds/click.wav"),
        "what the policy was asked"
    );
    let said = settled(&app).await;
    assert!(
        said.contains("asked for https://first.invalid/app/sounds/click.wav"),
        "{said}"
    );
    // A base the host gives wins over the page's.
    let runtime = Runtime::new();
    let script = ScriptHost::inline(&runtime, "app.luau", app_source("../up.wav"));
    script.fetch_policy(Says(asked.clone()));
    zgx_dom::install_fetcher_at(&script, "https://host.invalid/a/b/");
    let app = mount(&runtime, script);
    let said = settled(&app).await;
    assert!(
        said.contains("asked for https://host.invalid/a/up.wav"),
        "{said}"
    );
}

#[wasm_bindgen_test]
async fn a_network_failure_rejects_as_a_network_error() {
    let app = fetching("https://nothing-here.invalid/x");
    let said = settled(&app).await;
    assert!(said.starts_with("network: "), "{said}");
}

/// The page's `fetch` swapped for one that never answers until its signal
/// aborts, saying when it started and when it was aborted.
fn hanging_fetch() -> js_sys::Function {
    js_sys::Function::new_no_args(
        "const real = globalThis.fetch;
         globalThis.__zgx = { started: 0, aborted: 0, errors: 0 };
         const count = () => { globalThis.__zgx.errors += 1; };
         window.addEventListener('error', count);
         window.addEventListener('unhandledrejection', count);
         globalThis.fetch = (url, init) => new Promise((_, reject) => {
           globalThis.__zgx.started += 1;
           init.signal.addEventListener('abort', () => {
             globalThis.__zgx.aborted += 1;
             reject(new DOMException('aborted', 'AbortError'));
           });
         });
         return () => {
           globalThis.fetch = real;
           window.removeEventListener('error', count);
           window.removeEventListener('unhandledrejection', count);
         };",
    )
    .call0(&wasm_bindgen::JsValue::NULL)
    .unwrap()
    .unchecked_into()
}

fn count(what: &str) -> f64 {
    js_sys::eval(&format!("globalThis.__zgx.{what}"))
        .unwrap()
        .as_f64()
        .unwrap()
}

#[wasm_bindgen_test]
async fn a_cancelled_fetch_is_aborted_and_leaves_nothing_pending() {
    let restore = hanging_fetch();
    let runtime = Runtime::new();
    let script = ScriptHost::inline(
        &runtime,
        "app.luau",
        r#"
        local p = ui.fetch("https://slow.invalid/never")
        return ui.column {
            ui.button { on_click = function() p:cancel() end, "Cancel" },
            ui.text {
                id = "out",
                function()
                    if not p.done then return "loading" end
                    return if p.error then p.error.kind else "read"
                end,
            },
        }
        "#,
    );
    zgx_dom::install_fetcher(&script);
    let app = mount(&runtime, script);
    for _ in 0..100 {
        if count("started") > 0. {
            break;
        }
        tick().await;
    }
    assert_eq!(count("started"), 1.);
    assert_eq!(zgx_dom::__fetches_pending(), 1);
    let button: web_sys::HtmlElement = app
        .host
        .query_selector("button")
        .unwrap()
        .expect("the button")
        .dyn_into()
        .unwrap();
    button.click();
    let said = settled_text(&app).await;
    assert_eq!(said, "cancelled");
    for _ in 0..50 {
        if zgx_dom::__fetches_pending() == 0 {
            break;
        }
        tick().await;
    }
    assert_eq!(count("aborted"), 1.);
    assert_eq!(zgx_dom::__fetches_pending(), 0);
    // The aborted request settles into nothing Rust dropped: no closure
    // called after it was let go, no rejection left unhandled.
    for _ in 0..10 {
        tick().await;
    }
    assert_eq!(
        count("errors"),
        0.,
        "an error or unhandled rejection on the page"
    );
    restore.call0(&wasm_bindgen::JsValue::NULL).unwrap();
}

/// What the app shows after its button's "Cancel", once it isn't
/// "loading".
async fn settled_text(app: &App) -> String {
    for _ in 0..400 {
        tick().await;
        let text = app.host.text_content().unwrap_or_default();
        let text = text.strip_prefix("Cancel").unwrap_or(&text).to_owned();
        if !text.is_empty() && text != "loading" {
            return text;
        }
    }
    panic!("the fetch never settled");
}
