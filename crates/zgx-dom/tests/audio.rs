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
    // A short quiet hold (50 ms, not a second): every test waits for the
    // page's sound to go quiet, and the suite would otherwise wait a
    // second of silence each time.
    zgx_dom::__audio_set_hold(0.05);
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
        // Not wanted, and the context suspended (or never made): the next
        // sound starts fresh, its lead back at 120 ms.
        if state("running").as_bool() != Some(true)
            && state("context").as_string().as_deref() != Some("running")
        {
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
    let _timed = Timed::new("a_sound_plays_through_the_worklet_and_the_page_goes_quiet_after");
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
    // The tone (0.2 s) and the hold (50 ms here) pass: no more asked for, and the
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

/// The main thread held up while a sound plays, as by long frames. The
/// worklet runs dry only after an answer later than what it held when it
/// asked lasted: the main thread's lateness, never the queue's. A browser
/// promises nothing about how late a busy main thread answers (a loaded CI
/// runner was more than 118 ms late around a 50 ms hold), so that, not a
/// count of dry blocks, is what's held: dry blocks during the 50 ms holds
/// only with a late answer among them, and none with every answer in time.
/// Asks, waits, depths and counts are printed for the 50 ms and 150 ms
/// holds alike.
#[wasm_bindgen_test]
async fn a_busy_main_thread_runs_the_worklet_dry_only_when_it_answers_late() {
    let _timed = Timed::new("a_busy_main_thread_runs_the_worklet_dry_only_when_it_answers_late");
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
    let count = |key: &str| state(key).as_f64().unwrap_or(0.);
    let (before, late_before) = (count("underruns"), count("late"));
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
    web_sys::console::log_1(
        &format!(
            "depths (held, least, wait) through the 50 ms holds {}",
            depths()
        )
        .into(),
    );
    let at_50 = count("underruns") - before;
    let late_50 = count("late") - late_before;
    for _ in 0..3 {
        busy(150.);
        for _ in 0..10 {
            tick().await;
        }
    }
    let at_150 = count("underruns") - before - at_50;
    let late_150 = count("late") - late_before - late_50;
    web_sys::console::log_1(
        &format!(
            "underrun blocks: 6 holds of 50 ms {at_50} ({late_50} late answers), \
             3 of 150 ms {at_150} ({late_150} late answers)"
        )
        .into(),
    );
    // Dry only after a late answer, and never with every answer in time.
    assert!(
        at_50 == 0. || late_50 > 0.,
        "the worklet ran dry with every answer in time: {at_50} dry blocks; \
         depths (held, least, wait) {}",
        depths()
    );
    assert!(
        at_150 == 0. || late_150 > 0.,
        "the worklet ran dry with every answer in time (150 ms holds): {at_150}; {}",
        depths()
    );
}

/// A sound waking just as the worklet says it has drained: the suspend
/// that drain asks for and the resume the sound needs go in order, so the
/// context ends up running and the worklet answered, not suspended with
/// sound wanted.
#[wasm_bindgen_test]
async fn a_wake_just_after_a_drain_leaves_the_context_running() {
    let _timed = Timed::new("a_wake_just_after_a_drain_leaves_the_context_running");
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
    let _timed = Timed::new("a_second_host_given_sound_shares_the_engine");
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

/// Logs how long a test took when it ends: what the suite's time is
/// made of.
struct Timed(&'static str, f64);
impl Timed {
    fn new(name: &'static str) -> Timed {
        Timed(name, now_ms())
    }
}
impl Drop for Timed {
    fn drop(&mut self) {
        web_sys::console::log_1(&format!("{} took {:.0} ms", self.0, now_ms() - self.1).into());
    }
}
fn now_ms() -> f64 {
    web_sys::window().unwrap().performance().unwrap().now()
}

/// A page playing a 2 s tone round and round, with buttons to start and
/// stop it.
async fn looping() -> App {
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
        local v = nil
        return ui.column {{
            ui.text {{ function() return if tone.ready:get() then "ready" else "loading" end }},
            ui.button {{ on_click = function() v = audio.play(tone, {{ loop = true }}) end, "Play" }},
            ui.button {{ on_click = function() if v then v:stop() end end, "Stop" }},
        }}
        "#
    ));
    until("the sound loads", || text(&app).starts_with("ready")).await;
    // Answered twice since this play: playing, not waiting to begin (a
    // stall before the first block is a start, not a dry gap), and its
    // lead reported by this play's own asks, not a test's before.
    let posted = state("posted").as_f64().unwrap_or(0.);
    press(&app, 0);
    until("the worklet is answered twice", || {
        state("posted").as_f64().unwrap_or(0.) >= posted + 2.
    })
    .await;
    app
}

/// Presses the app's `n`th button.
fn press(app: &App, n: u32) {
    let button: web_sys::HtmlElement = app
        .host
        .query_selector_all("button")
        .unwrap()
        .item(n)
        .unwrap()
        .dyn_into()
        .unwrap();
    button.click();
}

/// The lead the worklet starts with: 120 ms at the context's rate.
fn base_mark() -> f64 {
    (0.12 * state("sampleRate").as_f64().unwrap()).round()
}

/// Holds the worklet's next answer back by `stall` frames, longer than it
/// holds: waits until it has run dry and the answer, late, has been let go
/// (`late` counts it as it lands), and then for its next ask, which reports
/// the lead it grew to.
async fn stall_past_its_queue(stall: f64) {
    let (dry, late, asked) = (
        state("underruns").as_f64().unwrap(),
        state("late").as_f64().unwrap(),
        state("asked").as_f64().unwrap(),
    );
    zgx_dom::__audio_stall_next_answer(stall as u32);
    until("the stall runs it dry", || {
        state("underruns").as_f64().unwrap() > dry
    })
    .await;
    until("the late answer lands", || {
        state("late").as_f64().unwrap() > late
    })
    .await;
    until("it asks again", || {
        state("asked").as_f64().unwrap() > asked + 1.
    })
    .await;
}

/// Waits `frames` audio frames' worth of time, and a little more.
async fn frames_pass(frames: f64) {
    let rate = state("sampleRate").as_f64().unwrap();
    let ticks = (frames / rate * 1000. / 10.).ceil() as usize + 10;
    for _ in 0..ticks {
        tick().await;
    }
}

/// A late answer (held back in the worklet, in audio frames, so it doesn't
/// hang on how busy the runner is) runs it dry once: its lead grows by half,
/// and the same stall then plays through.
#[wasm_bindgen_test]
async fn a_late_answer_grows_the_lead_and_the_same_stall_then_plays_through() {
    let _timed = Timed::new("a_late_answer_grows_the_lead_and_the_same_stall_then_plays_through");
    quiet().await;
    let app = looping().await;
    let base = base_mark();
    assert_eq!(state("mark").as_f64(), Some(base));
    // Longer than what it holds when it asks (under its mark): it runs dry.
    let stall = (base * 1.25).round();
    stall_past_its_queue(stall).await;
    let grown = (base * 1.5).round();
    assert_eq!(state("mark").as_f64(), Some(grown), "grown by half, once");
    // The same stall again: it holds at least its grown mark less a
    // quantum when it asks, more than the stall.
    let dry = state("underruns").as_f64().unwrap();
    zgx_dom::__audio_stall_next_answer(stall as u32);
    frames_pass(stall * 2.).await;
    assert_eq!(
        state("underruns").as_f64().unwrap(),
        dry,
        "the same stall ran it dry again; depths {}",
        depths()
    );
    assert_eq!(state("mark").as_f64(), Some(grown));
    press(&app, 1);
}

/// However often it runs dry, its lead stops at half a second.
#[wasm_bindgen_test]
async fn the_lead_never_passes_half_a_second() {
    let _timed = Timed::new("the_lead_never_passes_half_a_second");
    quiet().await;
    let app = looping().await;
    let cap = (0.5 * state("sampleRate").as_f64().unwrap()).round();
    // Stalls just past what it holds each time (1.25 times its lead): each
    // runs it dry and grows the lead by half, up to the cap, then one more
    // past the cap leaves it there.
    let mut stalls = 0;
    loop {
        let mark = state("mark").as_f64().unwrap();
        stall_past_its_queue((mark * 1.25).round()).await;
        let grown = state("mark").as_f64().unwrap();
        assert!(grown <= cap, "the lead {grown} passed {cap}");
        stalls += 1;
        if mark == cap {
            assert_eq!(grown, cap, "a stall past the cap left it at the cap");
            break;
        }
        assert!(stalls < 8, "the lead never reached the cap: {grown}");
    }
    press(&app, 1);
}

/// Grown, then the page goes quiet: the next sound, on a resumed context,
/// starts with the 120 ms lead again.
#[wasm_bindgen_test]
async fn the_lead_starts_over_on_a_resumed_context() {
    let _timed = Timed::new("the_lead_starts_over_on_a_resumed_context");
    quiet().await;
    let app = looping().await;
    let base = base_mark();
    stall_past_its_queue((base * 1.25).round()).await;
    assert!(state("mark").as_f64().unwrap() > base);
    press(&app, 1);
    quiet().await;
    until("the context is suspended", || {
        state("context").as_string().as_deref() == Some("suspended")
    })
    .await;
    let asked = state("asked").as_f64().unwrap();
    press(&app, 0);
    until("the worklet asks again", || {
        state("asked").as_f64().unwrap() > asked
    })
    .await;
    assert_eq!(state("mark").as_f64(), Some(base), "back to 120 ms");
    press(&app, 1);
}

/// A device that never runs dry keeps the 120 ms lead.
#[wasm_bindgen_test]
async fn a_device_that_never_runs_dry_keeps_its_lead() {
    let _timed = Timed::new("a_device_that_never_runs_dry_keeps_its_lead");
    quiet().await;
    let app = looping().await;
    let dry = state("underruns").as_f64().unwrap();
    for _ in 0..60 {
        tick().await;
    }
    // Growth needs a dry gap; with none, the lead is where it began.
    if state("underruns").as_f64().unwrap() == dry {
        assert_eq!(state("mark").as_f64(), Some(base_mark()));
    }
    press(&app, 1);
}
