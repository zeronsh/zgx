//! A Luau app's sound in the browser (the `audio` feature): zgx-audio's
//! mixer, the same as the desktop's, pulled by an audio worklet
//! (`js/zgx-audio.js`).
//!
//! The worklet is the clock. It asks for frames over its port when its
//! queue runs low, and the main thread mixes them (`Pull::fill`) and hands
//! them over, about 120 ms ahead. A port's messages aren't throttled in a
//! background tab, as timers and frames are. Nothing is made until the first
//! sound: then the `AudioContext` is, and the worklet starts asking. Once
//! the quiet hold has passed, `fill` says stop: the worklet stops asking,
//! and the context is suspended once its queue has played out. A silent
//! page runs no audio callback, posts no message and arms no timer.
//!
//! The page has one engine, whichever host asks first; every host given
//! sound (`install_audio`) plays through it, and each hears its own voices
//! end. A host given sound is kept for the page's life, as the engine is.
//!
//! A browser plays sound only after the page has been pressed: a one-shot
//! listener resumes the context on the first pointer or key press. One is
//! enough, as activation is sticky: once the page has seen a press, a
//! context made or resumed later needs no other. A strict
//! Content-Security-Policy needs `script-src blob:` for the worklet (made
//! from a Blob URL), or the worklet served as a file and named with
//! [`install_audio_at`].

use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use zgx_script::ScriptHost;
use zgx_script::zgx_audio::{Audio, Config, Pull};

#[wasm_bindgen(module = "/js/zgx-audio.js")]
extern "C" {
    fn zgx_audio_install(
        fill: &Closure<dyn FnMut(u32) -> JsValue>,
        rate: &Closure<dyn FnMut(f64)>,
        worklet_source: &str,
        worklet_url: Option<String>,
        ahead_seconds: f64,
    );
    fn zgx_audio_wake();
    fn zgx_audio_state() -> JsValue;
    fn zgx_audio_drain_and_wake(slow_suspend_ms: f64);
}

/// The worklet's source, made a Blob URL unless the page serves the file.
const WORKLET: &str = include_str!("../js/zgx-audio-worklet.js");

/// How far ahead of the output the main thread mixes, in seconds: past
/// what a long main-thread task (a big layout, a collection) takes, so the
/// worklet doesn't run dry.
const AHEAD: f64 = 0.12;

/// The page's sound: its one engine, once a script has asked, and the
/// hosts that play through it.
struct Page {
    engine: Option<(Audio, Pull)>,
    hosts: Vec<ScriptHost>,
    worklet: Option<String>,
}

thread_local! {
    static PAGE: RefCell<Option<Page>> = const { RefCell::new(None) };
}

/// Gives `host`'s scripts sound in the browser: their `audio` plays through
/// the page's one engine, made at the first call of any host's. The
/// worklet comes from a Blob URL (`script-src blob:` under a strict
/// Content-Security-Policy).
pub fn install_audio(host: &ScriptHost) {
    install(host, None);
}

/// As [`install_audio`], with the worklet served as the file at
/// `worklet_url` (a copy of zgx-dom's `js/zgx-audio-worklet.js`), for a
/// page whose Content-Security-Policy refuses `blob:` scripts. The first
/// host's worklet is the page's.
pub fn install_audio_at(host: &ScriptHost, worklet_url: &str) {
    install(host, Some(worklet_url.to_owned()));
}

/// What the page's sound is doing: `{ context, running, asked, posted,
/// underruns }` (`context` "none" before the first sound). For tests.
#[doc(hidden)]
pub fn __audio_state() -> JsValue {
    zgx_audio_state()
}

/// The worklet's drained message handled, as if the hold had passed, and a
/// sound waking in the same task, the suspend that drain asks for taking
/// `slow_suspend_ms`. For tests only, of the suspend and the resume keeping
/// their order: it changes the context's `suspend` for one call. Never call
/// it from an app.
#[doc(hidden)]
pub fn __audio_drain_and_wake(slow_suspend_ms: f64) {
    zgx_audio_drain_and_wake(slow_suspend_ms);
}

/// The page's engine, made now if no script has asked yet.
fn engine() -> Audio {
    PAGE.with_borrow_mut(|page| {
        let page = page.as_mut().expect("installed before a script asks");
        if let Some((audio, _)) = &page.engine {
            return audio.clone();
        }
        let (audio, pull) = Audio::pulled(Config::default(), zgx_audio_wake)
            .expect("zgx-audio's defaults are a config it runs");
        page.engine = Some((audio.clone(), pull));
        audio
    })
}

/// The worklet's ask for `frames`: the mixed block, and whether to go on.
fn fill(frames: u32) -> JsValue {
    let filled = PAGE.with_borrow_mut(|page| {
        let page = page.as_mut()?;
        let (audio, pull) = page.engine.as_mut()?;
        let mut block = vec![0.; frames as usize * pull.channels()];
        let going = pull.fill(&mut block);
        Some((block, going, audio.take_ended(), page.hosts.clone()))
    });
    let answer = js_sys::Array::new();
    match filled {
        Some((block, going, ended, hosts)) => {
            // Voices that ended in it: each host's `playing` states for them
            // turn false (a host knows its own and passes over the rest).
            // Told with the page let go of, as their scripts may play again.
            if !ended.is_empty() {
                for host in &hosts {
                    host.voices_ended(&ended);
                }
            }
            answer.push(&js_sys::Float32Array::from(&block[..]));
            answer.push(&JsValue::from_bool(going));
        }
        // No engine (it can't be asked for before one is made): an empty
        // block, and stop, so the worklet isn't left waiting.
        None => {
            answer.push(&js_sys::Float32Array::new_with_length(0));
            answer.push(&JsValue::FALSE);
        }
    }
    answer.into()
}

fn install(host: &ScriptHost, worklet: Option<String>) {
    let first = PAGE.with_borrow_mut(|page| match page {
        Some(page) => {
            if worklet.is_some() && worklet != page.worklet {
                web_sys::console::warn_1(
                    &"zgx-dom: install_audio_at: the page's worklet is the first host's".into(),
                );
            }
            page.hosts.push(host.clone());
            false
        }
        None => {
            *page = Some(Page {
                engine: None,
                hosts: vec![host.clone()],
                worklet: worklet.clone(),
            });
            true
        }
    });
    if first {
        let fill = Closure::new(fill);
        let rate = Closure::new(|rate: f64| {
            PAGE.with_borrow_mut(|page| {
                if let Some((_, pull)) = page.as_mut().and_then(|page| page.engine.as_mut()) {
                    pull.set_rate(rate as u32);
                }
            });
        });
        zgx_audio_install(&fill, &rate, WORKLET, worklet, AHEAD);
        // Kept for the page's life, as the engine is.
        fill.forget();
        rate.forget();
    }
    host.set_audio(engine);
}
