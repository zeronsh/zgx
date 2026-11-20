//! A name a host registers twice (`ScriptHost::action`, `function`,
//! `async_action`): the later handler answers, as ever; in a debug build a
//! second registration from another place in the host's code warns, naming
//! both, where a re-registration from the same place (a host rebuilt) and a
//! deliberate `replace_*` don't. These tests run in debug builds.
use std::{cell::RefCell, rc::Rc};
use zgx_script::{ScriptHost, Value};

/// A host whose script asks for its `"download"` action once as it loads.
fn host(runtime: &zgx::reactive::Runtime) -> ScriptHost {
    ScriptHost::inline(
        runtime,
        "app",
        r#"ui.invoke("download", "picture")
        return ui.div {}"#,
    )
}

/// The host's warnings about names registered twice.
fn twice(host: &ScriptHost) -> Vec<String> {
    host.warnings()
        .into_iter()
        .filter(|w| w.contains("registered again"))
        .collect()
}

/// A handler that writes `name` to `answered` when it runs: which of two
/// answered.
fn handler(answered: &Rc<RefCell<&'static str>>, name: &'static str) -> impl Fn(Value) + 'static {
    let answered = answered.clone();
    move |_| *answered.borrow_mut() = name
}

#[test]
fn a_name_registered_again_elsewhere_warns_naming_both_places() {
    let runtime = zgx::reactive::Runtime::new();
    let host = host(&runtime);
    let answered = Rc::new(RefCell::new(""));
    host.action("download", handler(&answered, "first"));
    let first_line = line!() - 1;
    host.action("download", handler(&answered, "second"));
    let second_line = line!() - 1;
    host.reload().unwrap();
    assert_eq!(
        *answered.borrow(),
        "second",
        "the later one answers, as ever"
    );
    let warnings = twice(&host);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let file = file!().rsplit('/').next().unwrap();
    for line in [first_line, second_line] {
        assert!(
            warnings[0].contains(&format!("{file}:{line}:")),
            "line {line} named: {warnings:?}"
        );
    }
    assert!(warnings[0].contains("replace_action"), "{warnings:?}");
    // In the debug strip too.
    let strip = host.lint_findings().get().unwrap_or_default();
    assert!(strip.contains("registered again"), "{strip}");
    // Each place once: the same two registrations again (a reload of the
    // host's own code) say nothing more.
    for _ in 0..2 {
        host.action("download", |_| {});
    }
    assert_eq!(
        twice(&host).len(),
        2,
        "the loop's site is a third place, once"
    );
}

#[test]
fn a_host_registering_from_the_same_place_again_is_silent() {
    let runtime = zgx::reactive::Runtime::new();
    let host = host(&runtime);
    // A host rebuilt (its registrations run again, from the same code).
    fn register(host: &ScriptHost) {
        host.action("download", |_| {});
        host.function("width", |_| Value::Num(1.));
        host.async_action("fetch", |_| async { Ok(Value::Nil) });
    }
    register(&host);
    register(&host);
    register(&host);
    assert!(twice(&host).is_empty(), "{:?}", twice(&host));
    assert!(host.lint_findings().get().is_none());
}

#[test]
fn replacing_on_purpose_never_warns() {
    let runtime = zgx::reactive::Runtime::new();
    let host = host(&runtime);
    let answered = Rc::new(RefCell::new(""));
    // Registered, then replaced on purpose, then the host rebuilt: neither
    // the replacement nor the registration it replaced says anything.
    for _ in 0..2 {
        host.action("download", handler(&answered, "first"));
        host.replace_action("download", handler(&answered, "second"));
    }
    host.replace_function("width", |_| Value::Num(2.));
    host.replace_async_action("fetch", |_| async { Ok(Value::Nil) });
    host.reload().unwrap();
    assert_eq!(*answered.borrow(), "second");
    assert!(twice(&host).is_empty(), "{:?}", twice(&host));
}

#[test]
fn functions_and_async_actions_registered_again_warn_too() {
    let runtime = zgx::reactive::Runtime::new();
    let host = host(&runtime);
    host.function("width", |_| Value::Num(1.));
    host.function("width", |_| Value::Num(2.));
    host.async_action("fetch", |_| async { Ok(Value::Nil) });
    host.async_action("fetch", |_| async { Ok(Value::Nil) });
    // A name is its kind's: an action and a function may share one.
    host.action("width", |_| {});
    let warnings = twice(&host);
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].contains("function \"width\""), "{warnings:?}");
    assert!(warnings[0].contains("replace_function"), "{warnings:?}");
    assert!(
        warnings[1].contains("async action \"fetch\""),
        "{warnings:?}"
    );
    assert!(warnings[1].contains("replace_async_action"), "{warnings:?}");
}
