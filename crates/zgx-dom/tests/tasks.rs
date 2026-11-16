//! A view's tasks and zgx's timers in a headless browser: the page's event
//! loop runs them, and an idle page has no timeout armed.
#![cfg(target_arch = "wasm32")]

use std::time::Duration;
use wasm_bindgen_test::*;
use zgx::{
    compose::{component, text_signal},
    reactive::Runtime,
};

wasm_bindgen_test_configure!(run_in_browser);

fn host() -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    host
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
async fn a_views_task_sleeps_and_finishes() {
    let runtime = Runtime::new();
    let done = runtime.signal(0);
    let (shown, set) = (done.clone(), done.clone());
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        component(move |cx| {
            // `cx.tasks()` has a runner: it panicked without one before.
            cx.tasks().spawn(async move {
                zgx::timer::sleep(Duration::from_millis(20)).await;
                set.set(1);
            });
            text_signal(move || format!("done {}", shown.get()))
        }),
    );
    let text = || host.text_content().unwrap_or_default();
    assert_eq!(text(), "done 0");
    sleep(5).await;
    assert!(zgx_dom::timer_armed(), "the sleep's timeout is armed");
    sleep(80).await;
    assert_eq!(text(), "done 1", "the task ran past its sleep");
    assert!(!zgx_dom::timer_armed(), "nothing waits: no timeout");
    mounted.unmount();
}

#[wasm_bindgen_test]
async fn an_unmounted_views_task_never_finishes() {
    let runtime = Runtime::new();
    let done = runtime.signal(false);
    let set = done.clone();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        component(move |cx| {
            cx.tasks().spawn(async move {
                zgx::timer::sleep(Duration::from_millis(30)).await;
                set.set(true);
            });
            text_signal(|| String::from("waiting"))
        }),
    );
    sleep(5).await;
    mounted.unmount();
    sleep(80).await;
    assert!(!done.get(), "the task went with its view");
    assert!(!zgx_dom::timer_armed(), "its sleep went too: no timeout");
}

#[wasm_bindgen_test]
async fn a_task_that_keeps_yielding_lets_the_page_run() {
    // Woken again as it runs, a task waits for the browser's next task: a
    // timeout of the page's still fires among its polls.
    let runtime = Runtime::new();
    let polls = runtime.signal(0_u32);
    let counted = polls.clone();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        component(move |cx| {
            cx.tasks().spawn(async move {
                loop {
                    counted.set(counted.get() + 1);
                    yield_once().await;
                }
            });
            text_signal(|| String::from("busy"))
        }),
    );
    sleep(20).await;
    assert!(polls.get() > 1, "it ran again: {}", polls.get());
    mounted.unmount();
}

/// Pending once, waking itself: a yield to the executor's next tick.
async fn yield_once() {
    let mut yielded = false;
    std::future::poll_fn(|cx| {
        if yielded {
            return std::task::Poll::Ready(());
        }
        yielded = true;
        cx.waker().wake_by_ref();
        std::task::Poll::Pending
    })
    .await;
}
