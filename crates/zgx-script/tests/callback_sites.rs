//! A panic in a host's Rust callback a script called (an action, a
//! function, an async action, the fetcher) notes where the host registered
//! it (`zgx::callback_site`), for the contained panic's report: the panic
//! may be deep in a library the callback used.
use std::panic::{AssertUnwindSafe, catch_unwind};
use zgx_script::{ScriptHost, testing::TestApp};

/// `script`'s button, `register`ed and clicked, then frames run: the
/// panic's noted kind and line.
fn noted(register: impl FnOnce(&ScriptHost) -> u32, script: &str) -> (&'static str, u32, u32) {
    zgx::callback_site::clear();
    let mut line = 0;
    let mut h = TestApp::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", script);
        line = register(&host);
        host
    });
    let caught = catch_unwind(AssertUnwindSafe(|| {
        h.click("go");
        h.frames(3);
    }));
    assert!(caught.is_err(), "the callback panicked");
    let (kind, at) = zgx::callback_site::take().expect("its site was noted");
    assert!(at.file().ends_with("callback_sites.rs"), "{}", at.file());
    (kind, at.line(), line)
}

#[test]
fn a_panicking_action_notes_where_it_was_registered() {
    let (kind, at, line) = noted(
        |host| {
            let line = line!() + 1;
            host.action("save", |_| panic!("deep in a library"));
            line
        },
        r#"return ui.button { id = "go", on_click = ui.action("save"), "Go" }"#,
    );
    assert_eq!((kind, at), ("a host action", line));
}

#[test]
fn a_panicking_function_notes_where_it_was_registered() {
    let (kind, at, line) = noted(
        |host| {
            let line = line!() + 1;
            host.function("rows", |_| panic!("deep in a library"));
            line
        },
        r#"return ui.button { id = "go", on_click = function() ui.call_sync("rows") end, "Go" }"#,
    );
    assert_eq!((kind, at), ("a host function", line));
}

#[test]
fn a_panicking_async_action_notes_where_it_was_registered_as_it_is_polled() {
    let (kind, at, line) = noted(
        |host| {
            let line = line!() + 1;
            host.async_action("load", |_| async move { panic!("deep in a library") });
            line
        },
        r#"return ui.button { id = "go", on_click = function() ui.call("load") end, "Go" }"#,
    );
    assert_eq!((kind, at), ("an async host action", line));
}

#[test]
fn a_panicking_fetcher_notes_where_it_was_registered() {
    let (kind, at, line) = noted(
        |host| {
            let line = line!() + 1;
            host.fetcher(|_: zgx_script::Request| async move { panic!("deep in a library") });
            line
        },
        r#"return ui.button { id = "go", on_click = function() ui.fetch("https://example.test/") end, "Go" }"#,
    );
    assert_eq!((kind, at), ("the host's fetcher", line));
}

#[test]
fn a_callback_that_returns_notes_nothing() {
    zgx::callback_site::clear();
    let mut h = TestApp::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"return ui.button { id = "go", on_click = ui.action("save"), "Go" }"#,
        );
        host.action("save", |_| {});
        host
    });
    h.click("go");
    h.frames(2);
    assert!(zgx::callback_site::take().is_none());
}
