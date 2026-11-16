//! A Luau app's `audio` in a headless browser: nothing made or posted while
//! silent, a play making the context and the worklet asking for blocks,
//! the context suspended once the quiet hold has passed and nothing asked
//! for after, and a sound's file fetched (a `blob:` URL of a WAV made
//! here) and decoded, its voice's `playing` turning false as it ends.
//! Chrome must let a context run without a press
//! (`--autoplay-policy=no-user-gesture-required`, webdriver.json).
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;
use zgx::reactive::Runtime;
use zgx_script::ScriptHost;
use zgx_script::zgx_audio::wav;

wasm_bindgen_test_configure!(run_in_browser);

async fn tick() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 10)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    zgx_dom::flush();
}

fn state(key: &str) -> JsValue {
    js_sys::Reflect::get(&zgx_dom::__audio_state(), &JsValue::from_str(key)).unwrap()
}

/// A `blob:` URL of 0.2 s of a tone, as a WAV.
fn tone_url() -> String {
    let samples: Vec<f32> = (0..9_600).map(|i| (i as f32 * 0.06).sin() * 0.25).collect();
    let bytes = wav::write(&samples, 1, 48_000, wav::Encoding::Float32).unwrap();
    js_sys::Function::new_with_args("bytes", "return URL.createObjectURL(new Blob([bytes]))")
        .call1(&JsValue::NULL, &js_sys::Uint8Array::from(&bytes[..]))
        .unwrap()
        .as_string()
        .unwrap()
}

struct App {
    host: web_sys::Element,
    _mounted: zgx_dom::Mounted,
    _script: ScriptHost,
}

fn mount(source: &str) -> App {
    let runtime = Runtime::new();
    let script = ScriptHost::inline(&runtime, "app.luau", source);
    zgx_dom::install_fetcher(&script);
    zgx_dom::install_audio(&script);
    script.reload().expect("the script loads");
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let mounted = zgx_dom::mount(&runtime, &host, script.view());
    App {
        host,
        _mounted: mounted,
        _script: script,
    }
}

async fn until(what: &str, done: impl Fn() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        tick().await;
    }
    panic!(
        "{what}: never, the sound's state {:?}",
        js_sys::JSON::stringify(&zgx_dom::__audio_state())
    );
}

/// Until the page's one engine (shared by every test here) is quiet: a
/// test before may have left a sound playing, or the hold not yet passed.
async fn quiet() {
    for _ in 0..800 {
        if state("running").as_bool() != Some(true) {
            return;
        }
        tick().await;
    }
    panic!("the page's sound never went quiet");
}

/// The worklet's queue as it asked, the last few times: (held, least held
/// since its ask before), in frames.
fn depths() -> String {
    js_sys::JSON::stringify(&state("depths"))
        .ok()
        .and_then(|s| s.as_string())
        .unwrap_or_default()
}

fn text(app: &App) -> String {
    app.host.text_content().unwrap_or_default()
}

#[wasm_bindgen_test]
async fn a_sound_plays_through_the_worklet_and_the_page_goes_quiet_after() {
    quiet().await;
    let url = tone_url();
    // The page's one engine may be left from a test before (suspended):
    // what's checked is that loading changes nothing of it.
    let (context, posted) = (state("context").as_string(), state("posted").as_f64());
    let app = mount(&format!(
        r#"
        local tone = audio.sound("{url}")
        local voice = ui.state("voice", false)
        local v = nil
        return ui.column {{
            ui.text {{ function() return if tone.ready:get() then "ready" else "loading" end }},
            ui.text {{
                function()
                    -- Read first: a play shows once it starts.
                    voice:get()
                    return if v and v.playing:get() then " playing" else " quiet"
                end,
            }},
            ui.button {{ on_click = function() v = audio.play(tone); voice:set(true) end, "Play" }},
        }}
        "#
    ));
    // Loading a sound (fetched and decoded) starts nothing: no context made
    // or resumed, nothing asked for or posted.
    until("the sound loads", || text(&app).starts_with("ready")).await;
    assert_eq!(
        state("context").as_string(),
        context,
        "loading made or woke a context"
    );
    assert_eq!(state("posted").as_f64(), posted, "loading posted blocks");
    assert_eq!(state("running").as_bool(), Some(false));
    let button: web_sys::HtmlElement = app
        .host
        .query_selector("button")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    button.click();
    for _ in 0..500 {
        if state("context").as_string().as_deref() != Some("none") {
            break;
        }
        tick().await;
    }
    for _ in 0..100 {
        if state("context").as_string().as_deref() == Some("running") {
            break;
        }
        tick().await;
    }
    assert_eq!(
        state("context").as_string().as_deref(),
        Some("running"),
        "the AudioContext didn't run: Chrome needs \
         --autoplay-policy=no-user-gesture-required in webdriver.json's \
         goog:chromeOptions args, as a test's click() isn't a trusted press"
    );
    until("the worklet is answered", || {
        state("posted").as_f64() > Some(0.)
    })
    .await;
    // The tone (0.2 s) and the hold (1 s) pass: no more asked for, and the
    // context suspended.
    until("the page goes quiet", || {
        state("running").as_bool() == Some(false)
            && state("context").as_string().as_deref() == Some("suspended")
    })
    .await;
    let posted = state("posted").as_f64();
    for _ in 0..30 {
        tick().await;
    }
    assert_eq!(
        state("posted").as_f64(),
        posted,
        "nothing posted once quiet"
    );
    assert!(!zgx_dom::timer_armed(), "no timer left armed");
    assert!(text(&app).contains("quiet"), "{}", text(&app));
}

/// The main thread held up while a sound plays, as by a long frame: with
/// blocks mixed 120 ms ahead, 50 ms holds never run the worklet dry. 150 ms
/// ones (past the lead) do, and fade rather than click; their count is
/// printed, not held to a number.
#[wasm_bindgen_test]
async fn a_busy_main_thread_doesnt_run_the_worklet_dry() {
    quiet().await;
    let samples: Vec<f32> = (0..96_000)
        .map(|i| (i as f32 * 0.06).sin() * 0.25)
        .collect();
    let bytes = wav::write(&samples, 1, 48_000, wav::Encoding::Float32).unwrap();
    let url =
        js_sys::Function::new_with_args("bytes", "return URL.createObjectURL(new Blob([bytes]))")
            .call1(&JsValue::NULL, &js_sys::Uint8Array::from(&bytes[..]))
            .unwrap()
            .as_string()
            .unwrap();
    let app = mount(&format!(
        r#"
        local tone = audio.sound("{url}")
        return ui.column {{
            ui.text {{ function() return if tone.ready:get() then "ready" else "loading" end }},
            ui.button {{ on_click = function() audio.play(tone) end, "Play" }},
        }}
        "#
    ));
    until("the sound loads", || text(&app).starts_with("ready")).await;
    let button: web_sys::HtmlElement = app
        .host
        .query_selector("button")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    button.click();
    until("the worklet is answered", || {
        state("posted").as_f64() > Some(0.)
    })
    .await;
    let hold = js_sys::Function::new_with_args(
        "ms",
        "const start = performance.now(); while (performance.now() - start < ms) {}",
    );
    let busy = |ms: f64| {
        hold.call1(&JsValue::NULL, &JsValue::from_f64(ms)).unwrap();
    };
    // Playing a while first: its first blocks in and reported.
    for _ in 0..20 {
        tick().await;
    }
    let before = state("underruns").as_f64().unwrap_or(0.);
    web_sys::console::log_1(
        &format!(
            "rate {:?}; depths before the holds {}",
            state("sampleRate").as_f64(),
            depths()
        )
        .into(),
    );
    for _ in 0..6 {
        busy(50.);
        for _ in 0..10 {
            tick().await;
        }
    }
    web_sys::console::log_1(&format!("depths through the 50 ms holds {}", depths()).into());
    let at_50 = state("underruns").as_f64().unwrap_or(0.) - before;
    for _ in 0..3 {
        busy(150.);
        for _ in 0..10 {
            tick().await;
        }
    }
    let at_150 = state("underruns").as_f64().unwrap_or(0.) - before - at_50;
    web_sys::console::log_1(
        &format!("underrun blocks: 6 holds of 50 ms {at_50}, 3 of 150 ms {at_150}").into(),
    );
    assert_eq!(
        at_50,
        0.,
        "a 50 ms hold ran the worklet dry; depths (held, least) {}",
        depths()
    );
}

/// A sound waking just as the worklet says it has drained: the suspend
/// that drain asks for and the resume the sound needs go in order, so the
/// context ends up running and the worklet answered, not suspended with
/// sound wanted.
#[wasm_bindgen_test]
async fn a_wake_just_after_a_drain_leaves_the_context_running() {
    quiet().await;
    let samples: Vec<f32> = (0..96_000)
        .map(|i| (i as f32 * 0.06).sin() * 0.25)
        .collect();
    let bytes = wav::write(&samples, 1, 48_000, wav::Encoding::Float32).unwrap();
    let url =
        js_sys::Function::new_with_args("bytes", "return URL.createObjectURL(new Blob([bytes]))")
            .call1(&JsValue::NULL, &js_sys::Uint8Array::from(&bytes[..]))
            .unwrap()
            .as_string()
            .unwrap();
    let app = mount(&format!(
        r#"
        local tone = audio.sound("{url}")
        return ui.column {{
            ui.text {{ function() return if tone.ready:get() then "ready" else "loading" end }},
            ui.button {{ on_click = function() audio.play(tone) end, "Play" }},
        }}
        "#
    ));
    until("the sound loads", || text(&app).starts_with("ready")).await;
    let button: web_sys::HtmlElement = app
        .host
        .query_selector("button")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    button.click();
    until("the worklet is answered", || {
        state("posted").as_f64() > Some(0.)
    })
    .await;
    zgx_dom::__audio_drain_and_wake(50.);
    for _ in 0..20 {
        tick().await;
    }
    assert_eq!(
        state("context").as_string().as_deref(),
        Some("running"),
        "suspended with sound wanted"
    );
    let posted = state("posted").as_f64().unwrap_or(0.);
    until("blocks are posted after the wake", || {
        state("posted").as_f64().unwrap_or(0.) > posted
    })
    .await;
}

/// Two hosts given sound share the page's one engine: the second's install
/// leaves the first playing.
#[wasm_bindgen_test]
async fn a_second_host_given_sound_shares_the_engine() {
    quiet().await;
    let url = tone_url();
    let source = format!(
        r#"
        local tone = audio.sound("{url}")
        return ui.column {{
            ui.text {{ function() return if tone.ready:get() then "ready" else "loading" end }},
            ui.button {{ on_click = function() audio.play(tone) end, "Play" }},
        }}
        "#
    );
    let first = mount(&source);
    let second = mount(&source);
    until("both load", || {
        text(&first).starts_with("ready") && text(&second).starts_with("ready")
    })
    .await;
    let posted = state("posted").as_f64().unwrap_or(0.);
    let button: web_sys::HtmlElement = first
        .host
        .query_selector("button")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    button.click();
    until("the first host's sound is answered", || {
        state("posted").as_f64().unwrap_or(0.) > posted
    })
    .await;
}
