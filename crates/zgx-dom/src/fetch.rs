//! A Luau app's transport in the browser (the `luau` feature): `ui.fetch`
//! through the page's `fetch` (`js/zgx-fetch.js`), as zgx-run's is through
//! its own HTTP stack on the desktop.
//!
//! The same rules as the desktop's apply, in the same places. A plugin
//! reaches the fetcher only with `Capability::Fetch` (zgx-script refuses
//! it otherwise, on every target). The app's `FetchPolicy` is asked of the
//! URL and each header the script set before the request goes. With a
//! policy, redirects are refused rather than followed, as a page can't
//! show each hop to ask it. A browser resolves names itself, so
//! `FetchPolicy::address` isn't asked here: the page's own origin rules
//! (CORS, `connect-src`) stand in for it.
//!
//! A relative URL resolves against the base the app was installed with,
//! the page's `document.baseURI` then unless the host gives one, never
//! where the page has since navigated. A network failure, a refused CORS
//! request or redirect, and an abort come back as the fetcher's error,
//! the script's promise rejecting with its message, as on the desktop. An
//! HTTP status (a 404) is a response, as there.

use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use zgx_script::{Body, Request, Response, ScriptHost, Url};

#[wasm_bindgen(module = "/js/zgx-fetch.js")]
extern "C" {
    #[wasm_bindgen(catch)]
    fn zgx_fetch_url(url: &str, base: &str) -> Result<Box<[JsValue]>, JsValue>;
    fn zgx_fetch(
        href: &str,
        method: &str,
        headers: Vec<String>,
        body: Option<Box<[u8]>>,
        redirect: &str,
    ) -> JsValue;
    fn zgx_fetches_pending() -> u32;
}

/// Gives `host` the page's `fetch` as its transport, relative URLs read
/// against the page's base URL as it is now (`document.baseURI`).
pub fn install_fetcher(host: &ScriptHost) {
    let base = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.base_uri().ok().flatten())
        .unwrap_or_default();
    install_fetcher_at(host, &base);
}

/// Gives `host` the page's `fetch` as its transport, relative URLs read
/// against `base` (the URL the app's files are served under).
pub fn install_fetcher_at(host: &ScriptHost, base: &str) {
    let base: Rc<str> = base.into();
    host.streaming_fetcher(move |request, body| {
        let base = base.clone();
        async move { fetch(request, &body, &base).await }
    });
}

/// Requests started and not yet settled.
#[doc(hidden)]
pub fn __fetches_pending() -> u32 {
    zgx_fetches_pending()
}

/// Aborts its request when dropped before the response is read: a future
/// the script let go of (a reload, a cancelled fetch) stops downloading.
struct Abort(Option<js_sys::Function>);

impl Drop for Abort {
    fn drop(&mut self) {
        if let Some(abort) = self.0.take() {
            let _ = abort.call0(&JsValue::NULL);
        }
    }
}

fn message(error: JsValue) -> String {
    error
        .as_string()
        .or_else(|| {
            error
                .dyn_ref::<js_sys::Error>()
                .map(|error| String::from(error.message()))
        })
        .unwrap_or_else(|| "the request failed".into())
}

async fn fetch(request: Request, body: &Body, base: &str) -> Result<Response, String> {
    let parts =
        zgx_fetch_url(&request.url, base).map_err(|_| format!("{}: not a URL", request.url))?;
    let part = |i: usize| {
        parts
            .get(i)
            .and_then(JsValue::as_string)
            .unwrap_or_default()
    };
    let (href, scheme, host) = (part(0), part(1), part(2));
    let port = part(3).parse::<u16>().ok();
    let policy = body.fetch_policy();
    if let Some(policy) = &policy {
        let caller = body.caller();
        policy.url(caller, &Url::new(href.clone(), scheme, host.clone(), port))?;
        for (name, _) in &request.headers {
            policy.header(caller, &host, name)?;
        }
    }
    let headers = request
        .headers
        .iter()
        .flat_map(|(name, value)| [name.clone(), value.clone()])
        .collect();
    let redirect = if policy.is_some() { "error" } else { "follow" };
    let started = zgx_fetch(
        &href,
        &request.method,
        headers,
        request.body.map(Vec::into_boxed_slice),
        redirect,
    );
    let get = |key: &str| js_sys::Reflect::get(&started, &JsValue::from_str(key));
    let promise: js_sys::Promise = get("promise")
        .ok()
        .and_then(|p| p.dyn_into().ok())
        .ok_or_else(|| "the request couldn't start".to_string())?;
    let mut abort = Abort(get("abort").ok().and_then(|f| f.dyn_into().ok()));
    let answer = JsFuture::from(promise).await.map_err(message)?;
    // Settled: nothing left to abort.
    abort.0 = None;
    let field = |key: &str| js_sys::Reflect::get(&answer, &JsValue::from_str(key)).ok();
    let status = field("status").and_then(|s| s.as_f64()).unwrap_or(0.) as u16;
    let flat: Vec<String> = field("headers")
        .map(|h| {
            js_sys::Array::from(&h)
                .iter()
                .filter_map(|v| v.as_string())
                .collect()
        })
        .unwrap_or_default();
    let (pairs, _) = flat.as_chunks::<2>();
    let headers = pairs
        .iter()
        .map(|[name, value]| (name.clone(), value.clone()))
        .collect();
    if let Some(bytes) = field("bytes") {
        body.write(&js_sys::Uint8Array::new(&bytes).to_vec());
    }
    Ok(Response {
        status,
        headers,
        body: Vec::new(),
    })
}
