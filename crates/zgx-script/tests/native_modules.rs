//! Native modules (`ScriptHost::load_module`): a library built with the
//! `zgx-module` crate registers functions and actions scripts call as a
//! host's own, its values the runtime's, counted not copied. The same
//! module linked in (`enter_module`) answers the same.
use std::{path::PathBuf, rc::Rc};
use zgx_script::{ScriptHost, Value, testing::TestApp};

const APP: &str = r#"
local function show(v): string
    if type(v) == "table" then
        local keys = {}
        for k in v do table.insert(keys, k) end
        table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
        local parts = {}
        for _, k in keys do table.insert(parts, `{k}={show(v[k])}`) end
        -- (Not braces: a text's braces are its template's.)
        return "[" .. table.concat(parts, ",") .. "]"
    end
    return tostring(v)
end
local echoed = ui.call_sync("fixture.echo", { 1, "two", true, { a = 1, b = { "x" } } })
local words = ui.call_sync("fixture.words", { "one two", "three", "four five six" })
local fields = ui.call_sync("fixture.fields", { drop = "b", a = 1, b = 2, c = "see" })
local before = ui.call_sync("fixture.kept")
ui.invoke("fixture.keep", { held = "by the module" })
local after = ui.call_sync("fixture.kept")
local shared = ui.call_sync("fixture.echo", ui.shared("rows"):get())
return ui.column {
    ui.text { id = "echo", show(echoed) },
    ui.text { id = "nil", show(ui.call_sync("fixture.echo")) },
    ui.text { id = "words", show(words) },
    ui.text { id = "fields", show(fields) },
    ui.text { id = "kept", `{show(before)} then {show(after)}` },
    ui.text { id = "shared", show(shared) },
}
"#;

/// The fixture as a library: cargo builds a dependency's `cdylib` with the
/// test's other dependencies, beside the test binary or (a build directory
/// per package) in the fixture's own.
fn library() -> PathBuf {
    let name = format!(
        "{}zgx_module_fixture{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let exe = std::env::current_exe().unwrap();
    let mut found: Vec<PathBuf> = Vec::new();
    for dir in exe.ancestors().skip(1).take(4) {
        found.push(dir.join(&name));
        found.push(dir.join("deps").join(&name));
        let builds = std::fs::read_dir(dir.join("zgx-module-fixture"))
            .into_iter()
            .flatten();
        found.extend(
            builds
                .flatten()
                .map(|build| build.path().join("out").join(&name)),
        );
    }
    found.retain(|path| path.is_file());
    // The newest, if an older build's is still there.
    found.sort_by_key(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok());
    found
        .pop()
        .unwrap_or_else(|| panic!("no {name} near {}", exe.display()))
}

fn rows() -> Value {
    Value::List(Rc::from(vec![
        Value::record([("id", Value::from("a")), ("n", Value::Num(1.))]),
        Value::record([("id", Value::from("b")), ("n", Value::Num(2.))]),
    ]))
}

fn app(load: impl FnOnce(&ScriptHost) + 'static) -> TestApp {
    TestApp::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.shared("rows", rows());
        load(&host);
        host
    })
}

fn answers(app: &mut TestApp) -> Vec<String> {
    assert_eq!(app.error(), None);
    ["echo", "nil", "words", "fields", "kept", "shared"]
        .map(|id| app.text(id))
        .to_vec()
}

#[test]
fn a_loaded_module_answers_scripts() {
    let mut app = app(|host| {
        // SAFETY: This workspace's own fixture.
        unsafe { host.load_module(library()) }.unwrap();
    });
    assert_eq!(
        answers(&mut app),
        [
            "[1=1,2=two,3=true,4=[a=1,b=[1=x]]]",
            "nil",
            "[first=one two,longest=four five six,words=6]",
            "[1=[key=a,value=1],2=[key=c,value=see],3=[key=drop,value=b]]",
            "nil then [held=by the module]",
            "[1=[id=a,n=1],2=[id=b,n=2]]",
        ]
    );
}

#[test]
fn a_module_linked_in_answers_the_same() {
    let mut linked = app(|host| {
        // SAFETY: The fixture's `module!` entry.
        unsafe { host.enter_module(zgx_module_fixture::zgx_module_v1) }.unwrap();
    });
    let mut loaded = app(|host| {
        // SAFETY: This workspace's own fixture.
        unsafe { host.load_module(library()) }.unwrap();
    });
    assert_eq!(answers(&mut linked), answers(&mut loaded));
}

/// A list a module hands back is the list it was given: the same `Rc`, so
/// state written with it changes nothing and copies nothing.
#[test]
fn values_cross_counted_not_copied() {
    let rows = rows();
    let Value::List(list) = &rows else {
        unreachable!()
    };
    let before = Rc::strong_count(list);
    let given = rows.clone();
    let mut app = TestApp::new(move |runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"
            local back = ui.shared("back")
            return ui.button { id = "go", w = 40, h = 20, "go", on_click = function()
                back:set(ui.call_sync("fixture.echo", ui.shared("rows"):get()))
            end }"#,
        );
        host.shared("rows", given);
        host.shared("back", Value::Nil);
        // SAFETY: This workspace's own fixture.
        unsafe { host.load_module(library()) }.unwrap();
        host
    });
    assert_eq!(app.error(), None);
    app.click("go");
    assert_eq!(app.error(), None);
    // Held by `rows` here and the state: the module kept none.
    assert!(Rc::strong_count(list) > before);
    drop(app);
    assert_eq!(
        Rc::strong_count(list),
        before,
        "a module's values are given back"
    );
}

#[test]
fn what_is_not_a_module_says_so() {
    let app = TestApp::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", "return ui.column {}");
        // SAFETY: Never loaded: there is no such file.
        let missing = unsafe { host.load_module("/nonexistent/module.so") };
        assert!(missing.unwrap_err().starts_with("/nonexistent/module.so: "));
        host
    });
    assert_eq!(app.error(), None);
}

/// A module's function called from the host's own Rust, as a script would.
#[test]
fn rust_calls_what_a_module_registered() {
    let app = TestApp::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", "return ui.column {}");
        // SAFETY: This workspace's own fixture.
        unsafe { host.load_module(library()) }.unwrap();
        let words = host.registered_function("fixture.words").unwrap();
        let counted = words(Value::List(Rc::from(vec![
            Value::from("a b"),
            Value::from("c"),
        ])));
        assert_eq!(counted.field("words"), Some(&Value::Num(3.)));
        let keep = host.registered_action("fixture.keep").unwrap();
        keep(Value::from("kept"));
        let kept = host.registered_function("fixture.kept").unwrap();
        assert_eq!(kept(Value::Nil), Value::from("kept"));
        assert!(host.registered_function("fixture.none").is_none());
        host
    });
    assert_eq!(app.error(), None);
}
