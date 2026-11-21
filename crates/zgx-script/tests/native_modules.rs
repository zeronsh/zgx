//! Native modules (`ScriptHost::load_module`): a library built with the
//! `zgx-module` crate registers functions and actions scripts call as a
//! host's own, its values the runtime's, counted not copied. The same
//! module linked in (`enter_module`) answers the same.
use std::{path::PathBuf, rc::Rc};
use zgx_script::List;
use zgx_script::Slice;
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
local hundred, wide = {}, {}
for i = 1, 100 do
    hundred[i] = i
    wide[`k{i}`] = i
end
local sum = ui.call_sync("fixture.sum", hundred)
local slice = ui.call_sync("fixture.slice", { list = hundred, from = 40, to = 43 })
local past = ui.call_sync("fixture.slice", { list = hundred, from = 99, to = 101 })
return ui.column {
    ui.text { id = "echo", show(echoed) },
    ui.text { id = "nil", show(ui.call_sync("fixture.echo")) },
    ui.text { id = "words", show(words) },
    ui.text { id = "fields", show(fields) },
    ui.text { id = "kept", `{show(before)} then {show(after)}` },
    ui.text { id = "shared", show(shared) },
    ui.text { id = "long", `{sum} {show(slice)} {show(past)} {show(ui.call_sync("fixture.wide", wide))}` },
}
"#;

/// The fixture as a library: cargo builds a dependency's `cdylib` with the
/// test's other dependencies, beside the test binary or (a build directory
/// per package) in the fixture's own.
fn library() -> PathBuf {
    library_named("zgx_module_fixture", "zgx-module-fixture")
}
fn library_named(library: &str, package: &str) -> PathBuf {
    let name = format!(
        "{}{library}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let exe = std::env::current_exe().unwrap();
    let mut found: Vec<PathBuf> = Vec::new();
    for dir in exe.ancestors().skip(1).take(4) {
        found.push(dir.join(&name));
        found.push(dir.join("deps").join(&name));
        let builds = std::fs::read_dir(dir.join(package)).into_iter().flatten();
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
    Value::List(zgx_script::Slice::from(vec![
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
    ["echo", "nil", "words", "fields", "kept", "shared", "long"]
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
            "5050 [1=41,2=42,3=43] nil [k57=57,last=k99,n=100,none=true]",
        ]
    );
}

#[test]
fn a_module_linked_in_answers_the_same() {
    let mut linked = app(|host| {
        // SAFETY: The fixture's `register!` entry.
        unsafe {
            host.enter_module(
                zgx_module_fixture::zgx_module_v1,
                &zgx_module_fixture::zgx_module_manifest,
            )
        }
        .unwrap();
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
    let before = Slice::strong_count(list);
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
    assert!(Slice::strong_count(list) > before);
    drop(app);
    assert_eq!(
        Slice::strong_count(list),
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
        let counted = words(Value::List(zgx_script::Slice::from(vec![
            Value::from("a b"),
            Value::from("c"),
        ])))
        .unwrap();
        assert_eq!(counted.field("words"), Some(&Value::Num(3.)));
        let keep = host.registered_action("fixture.keep").unwrap();
        keep(Value::from("kept")).unwrap();
        let kept = host.registered_function("fixture.kept").unwrap();
        assert_eq!(kept(Value::Nil), Ok(Value::from("kept")));
        assert!(host.registered_function("fixture.none").is_none());
        host
    });
    assert_eq!(app.error(), None);
}

/// A row kept folded: a module reads its fields as a script would, opened
/// for it, and hands it on as it is.
#[test]
fn a_module_reads_a_folded_row_and_passes_it_on() {
    struct Row;
    impl zgx_script::Unfold for Row {
        fn unfold(&self) -> Value {
            Value::record([
                ("id", Value::from("f1")),
                ("n", Value::Num(7.)),
                ("text", Value::from("folded")),
            ])
        }
    }
    let app = TestApp::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", "return ui.column {}");
        // SAFETY: This workspace's own fixture.
        unsafe { host.load_module(library()) }.unwrap();
        let folded = Value::folded(Rc::from("f1"), Row);
        let fields = host.registered_function("fixture.fields").unwrap();
        let listed = fields(folded.clone()).unwrap();
        let keys: Vec<&Value> = listed
            .list()
            .iter()
            .filter_map(|f| f.field("key"))
            .collect();
        assert_eq!(
            keys,
            [&Value::from("id"), &Value::from("n"), &Value::from("text")]
        );
        let echo = host.registered_function("fixture.echo").unwrap();
        assert!(matches!(echo(folded), Ok(Value::Folded(_))));
        host
    });
    assert_eq!(app.error(), None);
}

/// A string past 64 KiB, which the runtime holds in pieces: `text` reads it
/// whole, `str` (which reads in place) doesn't, and it passes through.
#[test]
fn a_module_reads_a_long_string() {
    let mut app = TestApp::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"
            local long = ui.state("long", string.rep("x", 70000) .. "end")
            local read = ui.call_sync("fixture.long", long:get())
            local short = ui.call_sync("fixture.long", "short")
            local back = ui.call_sync("fixture.echo", long:get())
            return ui.text { id = "out", `{read.len} {read.str} {short.len} {short.str} {#back} {string.sub(back, -3)}` }
            "#,
        );
        // SAFETY: This workspace's own fixture.
        unsafe { host.load_module(library()) }.unwrap();
        host
    });
    assert_eq!(app.error(), None);
    assert_eq!(app.text("out"), "70003 false 5 true 70003 end");
}

/// The last hold on a value a module kept is given back to the runtime as
/// the module lets go of it.
#[test]
fn a_module_frees_what_it_held_last() {
    let text: Rc<str> = Rc::from("held");
    let list = List::from(vec![Value::Str(text.clone().into())]);
    let given = list.clone();
    let app = TestApp::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", "return ui.column {}");
        // SAFETY: This workspace's own fixture.
        unsafe { host.load_module(library()) }.unwrap();
        let keep = host.registered_action("fixture.keep").unwrap();
        let weak = List::downgrade(&given);
        keep(Value::List(given)).unwrap();
        assert_eq!(weak.strong_count(), 2, "held here and by the module");
        keep(Value::Nil).unwrap();
        assert_eq!(weak.strong_count(), 1);
        host
    });
    assert_eq!(app.error(), None);
    drop(app);
    assert_eq!(List::strong_count(&list), 1);
    // Its item, held by the list alone: freed with it, by the runtime.
    drop(list);
    assert_eq!(Rc::strong_count(&text), 1);
}

/// A module's state is its thread's: two hosts on a thread share it, and a
/// host on another thread has its own.
#[test]
fn a_modules_state_is_one_per_thread() {
    let kept = |keep: Option<&str>| {
        let mut seen = Value::Nil;
        let app = TestApp::new(|runtime| {
            let host = ScriptHost::inline(runtime, "app", "return ui.column {}");
            // SAFETY: This workspace's own fixture.
            unsafe { host.load_module(library()) }.unwrap();
            if let Some(keep) = keep {
                host.registered_action("fixture.keep").unwrap()(Value::from(keep)).unwrap();
            }
            seen = host.registered_function("fixture.kept").unwrap()(Value::Nil).unwrap();
            host
        });
        assert_eq!(app.error(), None);
        seen
    };
    assert_eq!(kept(Some("here")), Value::from("here"));
    // Another host, the same thread: the same state.
    assert_eq!(kept(None), Value::from("here"));
    // Another thread: its own, and this thread's as it was.
    let there = std::thread::spawn(move || {
        (
            kept(None) == Value::Nil,
            kept(Some("there")) == Value::from("there"),
        )
    });
    assert_eq!(there.join().unwrap(), (true, true));
    assert_eq!(kept(None), Value::from("here"));
}

/// The fixture written with `zgx_module::module`, as a library.
fn typed_library() -> PathBuf {
    library_named("zgx_module_fixture_typed", "zgx-module-fixture-typed")
}

/// What each line of `script` gives or raises, as a text for each: a
/// module's answers and its errors, read the same way.
fn tried(lines: &[&str], load: impl FnOnce(&ScriptHost) + 'static) -> Vec<String> {
    let mut script = String::from(
        r#"
        local function show(v): string
            if type(v) == "table" then
                local keys = {}
                for k in v do table.insert(keys, k) end
                table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
                local parts = {}
                for _, k in keys do table.insert(parts, `{k}={show(v[k])}`) end
                return "[" .. table.concat(parts, ",") .. "]"
            end
            return tostring(v)
        end
        local function try(f): string
            local ok, got = pcall(f)
            if ok then return show(got) end
            -- (Not braces: a text's braces are its template's.)
            return "error: " .. string.gsub(tostring(got), "[{}]", "")
        end
        local typed = ui.module("typed")
        local out = {}
        "#,
    );
    for line in lines {
        script.push_str(&format!(
            "table.insert(out, try(function() return {line} end))\n"
        ));
    }
    script.push_str(
        r#"
        local texts = {}
        for i, text in out do texts[i] = ui.text { id = `out{i}`, text } end
        return ui.column(texts)
        "#,
    );
    let mut app = TestApp::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", script);
        load(&host);
        host
    });
    assert_eq!(app.error(), None);
    (1..=lines.len())
        .map(|i| app.text(&format!("out{i}")))
        .collect()
}
fn load_typed(host: &ScriptHost) {
    // SAFETY: This workspace's own fixture.
    unsafe { host.load_module(typed_library()) }.unwrap();
}

/// A module written with the macros: its functions as members of its
/// table, each with the arguments its Rust function takes, and by name.
#[test]
fn a_typed_module_answers_as_a_table_and_by_name() {
    let got = tried(
        &[
            r#"typed.count_words("one two  three")"#,
            r#"ui.call_sync("typed.count_words", "one two")"#,
            r#"typed.join("a", "b")"#,
            r#"typed.join("a", "b", "-")"#,
            r#"typed.find({ id = "abc" })"#,
            r#"typed.find({ id = "abc", limit = 1 })"#,
            r#"typed.find({ id = "" })"#,
            r#"typed.sum({ 1, 2, 3.5 })"#,
            r#"typed.dearest({ tea = 3, cake = 4.5, milk = 1 })"#,
            r#"typed.nothing()"#,
            r#"typed.doubled(21)"#,
            r#"ui.call_sync("other.doubled", -4)"#,
            r#"typed.checked(7)"#,
            r#"typed.any({ 1, "two" })"#,
            r#"typed.long(string.rep("x", 70000))"#,
            r#"typed.kept()"#,
            r#"(function() ui.invoke("typed.keep", "held") ui.invoke("typed.keep", { a = 1 }) return typed.kept() end)()"#,
            r#"typed.rows({ { id = "a", text = "bc", n = 2, done = true }, { id = "d", text = "", n = 0.5, done = false } })"#,
        ],
        load_typed,
    );
    assert_eq!(
        got,
        [
            "3",
            "2",
            "a b",
            "a-b",
            "[id=ABC,n=3,tags=[1=one,2=two]]",
            "[id=ABC,n=3,tags=[1=one]]",
            "nil",
            "6.5",
            "cake",
            "nil",
            "42",
            "-8",
            "7",
            "[1=1,2=two]",
            "70000",
            "[2=0]",
            "[1=[a=1],2=2]",
            "7.5",
        ]
    );
}

/// What a script passed wrongly is an error raised in it, at its line,
/// naming the function, its module, the argument and what it must be.
#[test]
fn a_wrong_argument_is_an_error_that_says_where() {
    let got = tried(
        &[
            r#"typed.count_words(5)"#,
            r#"ui.call_sync("typed.count_words", { "a" })"#,
            r#"typed.join("a")"#,
            r#"typed.join("a", "b", 3)"#,
            r#"typed.find({ id = 7 })"#,
            r#"typed.find({ id = "x", limit = 1.5 })"#,
            r#"typed.find("x")"#,
            r#"typed.sum({ 1, "two", 3 })"#,
            r#"typed.sum(4)"#,
            r#"typed.dearest({ tea = "dear" })"#,
            r#"typed.doubled(2.5)"#,
            r#"typed.checked(300)"#,
            r#"typed.checked(13)"#,
            r#"ui.invoke("typed.must", 5)"#,
            r#"ui.invoke("typed.must", "")"#,
            r#"ui.call_sync("other.doubled", "x")"#,
            r#"typed.rows({ { id = "a", text = "b", n = 1, done = true }, { id = "a", n = 1, done = true } })"#,
            r#"ui.module("none")"#,
        ],
        load_typed,
    );
    // (An error's first line; the script's own lines follow it.)
    let said = |at: usize, what: &str| {
        let first = got[at].lines().next().unwrap_or("");
        assert!(
            first.starts_with("error: ") && first.ends_with(what),
            "{}: {first:?} doesn't end with {what:?}",
            at + 1,
        );
    };
    said(
        0,
        "typed.count_words (module `typed`): argument 1 (text) must be a string, not a number",
    );
    said(
        1,
        "typed.count_words (module `typed`): argument 1 (text) must be a string, not a list",
    );
    said(
        2,
        "typed.join (module `typed`): argument 2 (b) must be a string, not nil",
    );
    said(
        3,
        "typed.join (module `typed`): argument 3 (sep) must be a string or nil, not a number",
    );
    said(
        4,
        "typed.find (module `typed`): argument 1 (query) field `id` must be a string, not a number",
    );
    said(
        5,
        "argument 1 (query) field `limit` must be a whole number from 0 to 4294967295 or nil, not 1.5",
    );
    said(
        6,
        "typed.find (module `typed`): argument 1 (query) must be a map (Query), not a string",
    );
    said(
        7,
        "typed.sum (module `typed`): argument 1 (numbers) item 2 must be a number, not a string",
    );
    said(
        8,
        "typed.sum (module `typed`): argument 1 (numbers) must be a list, not a number",
    );
    said(
        9,
        "typed.dearest (module `typed`): argument 1 (prices) field `tea` must be a number, not a string",
    );
    said(
        10,
        "other.doubled (module `typed`): argument 1 (n) must be a whole number from -2147483648 to 2147483647, not 2.5",
    );
    said(
        11,
        "typed.checked (module `typed`): argument 1 (n) must be a whole number from 0 to 255, not 300",
    );
    said(12, "typed.checked (module `typed`): is unlucky");
    said(
        13,
        "typed.must (module `typed`): argument 1 (text) must be a string, not a number",
    );
    said(14, "typed.must (module `typed`): was given nothing to do");
    said(
        15,
        "other.doubled (module `typed`): argument 1 (n) must be a whole number, not a string",
    );
    said(
        16,
        "argument 1 (rows) item 2 field `text` must be a string, not nil",
    );
    said(17, "no module named `none`: the app loaded `typed`");
    // Raised in the script, as its other errors are: its lines follow.
    assert!(got[0].contains("\tapp:"), "{:?}", got[0]);
}

/// An action bound to a control fails as a handler's error does: reported,
/// naming it, with the app still running.
#[test]
fn a_bound_action_that_fails_is_reported() {
    let mut app = TestApp::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"return ui.button { id = "go", "Go", on_click = ui.action("typed.must", 5) }"#,
        );
        load_typed(&host);
        host
    });
    assert_eq!(app.error(), None);
    app.click("go");
    let error = app.error().expect("the action's failure");
    assert!(
        error.ends_with(
            "typed.must (module `typed`): argument 1 (text) must be a string, not a number"
        ),
        "{error}"
    );
}

/// Loaded and linked in, a typed module answers the same.
#[test]
fn a_typed_module_linked_in_answers_the_same() {
    let lines = [
        r#"typed.join("a", "b", "+")"#,
        r#"typed.find({ id = "q" })"#,
        r#"typed.count_words(false)"#,
    ];
    let linked = tried(&lines, |host| {
        use zgx_module_fixture_typed::typed;
        // SAFETY: The typed fixture's own entry and manifest.
        unsafe { host.enter_module(typed::zgx_module_v1, &typed::zgx_module_manifest) }.unwrap();
    });
    assert_eq!(linked, tried(&lines, load_typed));
    assert_eq!(linked[0], "a+b");
}

/// A module's manifest says what it registers. Read from the library
/// loaded and from its file, it is one text; and a module registering
/// anything else than its manifest says is refused, with nothing of it
/// left registered.
#[test]
fn a_module_is_held_to_its_manifest() {
    use zgx_module::abi;
    use zgx_module_fixture_typed::typed;
    let file = std::fs::read(typed_library()).unwrap();
    let in_file = abi::manifest_in(&file).expect("the library's file holds its manifest");
    assert_eq!(abi::manifest_in(&typed::zgx_module_manifest), Some(in_file));
    let read: serde_json::Value = serde_json::from_str(in_file).unwrap();
    assert_eq!(read["name"], "typed");
    assert_eq!(
        read["functions"]["typed.join"]["args"][2][1]["optional"],
        "string"
    );
    assert_eq!(read["functions"]["other.doubled"]["member"], "doubled");
    assert_eq!(read["actions"]["typed.keep"]["args"][0][0], "value");
    assert_eq!(read["records"]["Query"]["limit"]["optional"], "number");

    // A manifest damaged anywhere is no manifest: a byte of its JSON, its
    // length, its end.
    let whole = typed::zgx_module_manifest.to_vec();
    for at in [abi::MANIFEST_HEAD + 5, 20, whole.len() - 3] {
        let mut damaged = whole.clone();
        damaged[at] ^= 1;
        assert_eq!(abi::manifest_in(&damaged), None, "byte {at}");
    }
    // The magic alone, as a program that names it holds it, is none either;
    // the manifest after it is still found.
    let mut after = abi::MANIFEST_MAGIC.to_vec();
    after.extend_from_slice(b"not a manifest");
    assert_eq!(abi::manifest_in(&after), None);
    after.extend_from_slice(&whole);
    assert_eq!(abi::manifest_in(&after), Some(in_file));

    let refused = |json: String| {
        let mut why = String::new();
        let mut app = TestApp::new(|runtime| {
            let host = ScriptHost::inline(
                runtime,
                "app",
                r#"return ui.text { id = "out", tostring(pcall(ui.call_sync, "typed.count_words", "a b")) }"#,
            );
            let manifest = abi::manifest_bytes(&json);
            // SAFETY: The typed fixture's entry, with a manifest that isn't its own.
            why = unsafe { host.enter_module(typed::zgx_module_v1, &manifest) }
                .expect_err("a module that isn't what its manifest says");
            assert!(host.module_manifests().is_empty());
            host
        });
        assert_eq!(app.error(), None);
        assert_eq!(app.text("out"), "false", "nothing of it is registered");
        why
    };
    let less = in_file.replace(
        r#""typed.nothing":{"member":"nothing","args":[],"returns":"nil"},"#,
        "",
    );
    assert_ne!(less, in_file);
    assert_eq!(
        refused(less),
        "it registers a function `typed.nothing` its manifest doesn't have"
    );
    let more = in_file.replace(
        r#""functions":{"#,
        r#""functions":{"typed.absent":{"args":[],"returns":"nil"},"#,
    );
    assert_eq!(
        refused(more),
        "its manifest has a function `typed.absent` it doesn't register"
    );
    assert_eq!(
        refused(in_file.replace(r#""abi":1"#, r#""abi":2"#)),
        "its manifest is for another module interface than this runtime's (1)"
    );
}

/// A module built for release and stripped still says what it is: its
/// manifest is in the file, in read-only data, reached by its symbol, and
/// the library loads and answers.
#[test]
fn a_stripped_release_build_keeps_its_manifest() {
    use std::process::Command;
    use zgx_module::abi;
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // A target of its own: this runs inside cargo's, and builds another
    // profile.
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace.join("target"), PathBuf::from)
        .join("module-release");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let built = Command::new(cargo)
        .current_dir(&workspace)
        .args([
            "build",
            "--release",
            "--locked",
            "-p",
            "zgx-module-fixture-typed",
        ])
        .arg("--target-dir")
        .arg(&target)
        .output()
        .expect("cargo runs");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let name = format!(
        "{}zgx_module_fixture_typed{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    // (A copy: stripped in place, cargo would see its own output changed.)
    let library = target.join(format!("stripped-{name}"));
    std::fs::copy(target.join("release").join(&name), &library).unwrap();
    // Windows keeps a release library's symbols in a file beside it, so
    // there is nothing to strip there.
    if cfg!(unix) {
        let flag = if cfg!(target_os = "macos") {
            "-x"
        } else {
            "--strip-all"
        };
        let stripped = Command::new("strip")
            .arg(flag)
            .arg(&library)
            .output()
            .expect("strip runs");
        assert!(
            stripped.status.success(),
            "{}",
            String::from_utf8_lossy(&stripped.stderr)
        );
    }
    let file = std::fs::read(&library).unwrap();
    let in_file = abi::manifest_in(&file).expect("the stripped file holds the manifest");
    assert_eq!(
        Some(in_file),
        abi::manifest_in(&zgx_module_fixture_typed::typed::zgx_module_manifest)
    );
    // Once in the file: the scan can't have met another's.
    let magic = file
        .windows(abi::MANIFEST_MAGIC.len())
        .filter(|at| *at == abi::MANIFEST_MAGIC)
        .count();
    assert_eq!(magic, 1);
    // In read-only data, by its exported symbol.
    if cfg!(target_os = "linux") {
        let symbols = Command::new("nm")
            .args(["-D", "--defined-only"])
            .arg(&library)
            .output()
            .expect("nm runs");
        let symbols = String::from_utf8_lossy(&symbols.stdout).into_owned();
        let of = |name: &str| {
            symbols
                .lines()
                .find(|line| line.ends_with(name))
                .unwrap_or_else(|| panic!("no {name} in:\n{symbols}"))
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_owned()
        };
        assert_eq!(of(" zgx_module_manifest_typed"), "R", "read-only data");
        assert_eq!(of(" zgx_module_v1_typed"), "T");
    }
    if cfg!(target_os = "macos") {
        let symbols = Command::new("nm")
            .args(["-gUm"])
            .arg(&library)
            .output()
            .expect("nm runs");
        let symbols = String::from_utf8_lossy(&symbols.stdout).into_owned();
        let manifest = symbols
            .lines()
            .find(|line| line.ends_with("_zgx_module_manifest_typed"))
            .unwrap_or_else(|| panic!("no manifest symbol in:\n{symbols}"));
        assert!(manifest.contains("(__TEXT,__const)"), "{manifest}");
    }
    // And it loads (the manifest read again through its symbol, and held
    // to) and answers.
    let got = tried(&[r#"typed.join("still", "here")"#], move |host| {
        // SAFETY: This workspace's own fixture, built above.
        unsafe { host.load_module(&library) }.unwrap();
    });
    assert_eq!(got, ["still here"]);
}

/// A C compiler's output of `source` (and the module header), built into
/// `out`: `cc` as the system has it.
#[cfg(unix)]
fn compiled(source: &str, out: &std::path::Path, more: &[&str]) {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let built = std::process::Command::new("cc")
        .args(["-std=c11", "-O2", "-Wall", "-Wextra", "-Werror"])
        .args(more)
        .arg("-I")
        .arg(here.join("../zgx-module/include"))
        .arg("-I")
        .arg(out.parent().unwrap())
        .arg(here.join("tests/c_module").join(source))
        .arg("-o")
        .arg(out)
        .output()
        .expect("a C compiler (`cc`) runs");
    assert!(
        built.status.success(),
        "{source}:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
}
/// A folder for what the C tests build, under cargo's own.
#[cfg(unix)]
fn c_out(name: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("c-module-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `zgx_module.h` is written by hand from `zgx_module::abi`: every size,
/// offset and number in it is Rust's, as a C compiler lays it out.
#[cfg(unix)]
#[test]
fn the_c_header_is_the_rust_interface() {
    use std::mem::offset_of;
    use zgx_module::abi::{self, Api, Bytes, Registrar};
    let dir = c_out("layout");
    let probe = dir.join("layout");
    compiled("layout.c", &probe, &[]);
    let printed = std::process::Command::new(&probe).output().unwrap();
    let printed = String::from_utf8(printed.stdout).unwrap();
    let c: std::collections::HashMap<&str, &str> = printed
        .lines()
        .filter_map(|line| line.split_once(' '))
        .collect();
    let number = |name: &str| -> usize {
        c.get(name)
            .unwrap_or_else(|| panic!("the header's probe prints no {name}"))
            .parse()
            .unwrap()
    };
    let word = size_of::<usize>();
    let expected = [
        ("ZGX_MODULE_ABI", abi::ABI as usize),
        ("ZGX_COUNT_BEFORE", abi::COUNT_BEFORE),
        ("ZGX_MANIFEST_HEAD", abi::MANIFEST_HEAD),
        ("sum", abi::manifest_sum(br#"{"name":"c"}"#) as usize),
        ("sizeof(zgx_value)", abi::VALUE_SIZE),
        ("zgx_value.kind", 0),
        ("zgx_value.boolean", abi::BOOL_AT),
        ("zgx_value.held", abi::HELD_AT),
        ("zgx_value.len", abi::HELD_AT + word),
        ("sizeof(zgx_field)", abi::FIELD_SIZE),
        ("zgx_field.key", abi::FIELD_KEY_AT),
        ("zgx_field.key_len", abi::FIELD_KEY_AT + word),
        ("zgx_field.value", abi::FIELD_VALUE_AT),
        ("sizeof(zgx_bytes)", size_of::<Bytes>()),
        ("zgx_bytes.ptr", offset_of!(Bytes, ptr)),
        ("zgx_bytes.len", offset_of!(Bytes, len)),
        ("sizeof(zgx_api)", size_of::<Api>()),
        ("zgx_api.abi", offset_of!(Api, abi)),
        ("zgx_api.size", offset_of!(Api, size)),
        ("zgx_api.string", offset_of!(Api, string)),
        ("zgx_api.list", offset_of!(Api, list)),
        ("zgx_api.map", offset_of!(Api, map)),
        ("zgx_api.slice", offset_of!(Api, slice)),
        ("zgx_api.clone", offset_of!(Api, clone)),
        ("zgx_api.release", offset_of!(Api, release)),
        ("zgx_api.open", offset_of!(Api, open)),
        ("zgx_api.flat", offset_of!(Api, flat)),
        ("sizeof(zgx_registrar)", size_of::<Registrar>()),
        ("zgx_registrar.size", offset_of!(Registrar, size)),
        ("zgx_registrar.host", offset_of!(Registrar, host)),
        ("zgx_registrar.function", offset_of!(Registrar, function)),
        (
            "zgx_registrar.function_taking",
            offset_of!(Registrar, function_taking),
        ),
        ("zgx_registrar.action", offset_of!(Registrar, action)),
        ("zgx_registrar.state", offset_of!(Registrar, state)),
    ];
    for (name, rust) in expected {
        assert_eq!(number(name), rust, "{name}");
    }
    // Every line the probe prints is held to something here: a field added
    // to the header is added to this test.
    assert_eq!(c.len(), expected.len() + 2, "{printed}");
    let kinds = [
        abi::NIL,
        abi::BOOL,
        abi::NUM,
        abi::STR,
        abi::TEXT,
        abi::LIST,
        abi::MAP,
        abi::FOLDED,
    ]
    .map(|kind| kind.to_string())
    .join(" ");
    assert_eq!(c["kinds"], kinds);
    assert_eq!(
        c["statuses"],
        format!("{} {} {}", abi::DONE, abi::FAILED, abi::REFUSED_ABI)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// A module written in C against the header alone loads and answers as one
/// written in Rust does: its manifest held to, its values read in place,
/// its failures raised.
#[cfg(unix)]
#[test]
fn a_module_written_in_c_answers_scripts() {
    use zgx_module::abi;
    let dir = c_out("words");
    let json = r#"{"zgx_module":1,"abi":1,"name":"cwords","version":"1.0.0","records":{"Row":{"id":"string","n":{"optional":"number"}}},"functions":{"cwords.count":{"args":[["text","string"]],"returns":"number"},"cwords.total":{"args":[["numbers",{"list":"number"}],["scale",{"optional":"number"}]],"returns":"number"},"cwords.describe":{"args":[["row",{"record":"Row"}]],"returns":{"map":"any"}},"cwords.kept":{"args":[],"returns":"any"}},"actions":{"cwords.keep":{"args":[["value","any"]]}}}"#;
    // (What a tool of the author's would write: the manifest's bytes as a C
    // array.)
    let bytes: Vec<String> = abi::manifest_bytes(json)
        .iter()
        .map(|byte| byte.to_string())
        .collect();
    std::fs::write(
        dir.join("manifest.h"),
        format!("ZGX_MODULE_MANIFEST(cwords) = {{{}}};\n", bytes.join(",")),
    )
    .unwrap();
    let library = dir.join(format!(
        "{}cwords{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    compiled("words.c", &library, &["-shared", "-fPIC"]);
    let load = {
        let library = library.clone();
        move |host: &ScriptHost| {
            // SAFETY: The C module this test just built.
            unsafe { host.load_module(&library) }.unwrap();
            assert_eq!(&*host.module_manifests()[0], json);
        }
    };
    let script = r#"
        local function show(v): string
            if type(v) == "table" then
                local keys = {}
                for k in v do table.insert(keys, k) end
                table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
                local parts = {}
                for _, k in keys do table.insert(parts, `{k}={show(v[k])}`) end
                return "[" .. table.concat(parts, ",") .. "]"
            end
            return tostring(v)
        end
        local words = ui.module("cwords")
        local held = { "kept", "by C" }
        ui.invoke("cwords.keep", held)
        local failed = select(2, pcall(words.count, 5))
        return ui.column {
            ui.text { id = "count", show(words.count("a module  in C")) },
            ui.text { id = "name", show(ui.call_sync("cwords.count", "by name")) },
            ui.text { id = "total", `{words.total({ 1, 2, 3.5 })} {words.total({ 1, 2 }, 10)}` },
            ui.text { id = "describe", show(words.describe({ id = "r1", n = 41, zzz = true })) },
            ui.text { id = "kept", show(words.kept()) },
            ui.text { id = "failed", (string.gsub(string.match(tostring(failed), "[^\n]*"), "[{}]", "")) },
        }
    "#;
    let mut app = TestApp::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", script);
        load(&host);
        host
    });
    assert_eq!(app.error(), None);
    assert_eq!(app.text("count"), "4");
    assert_eq!(app.text("name"), "2");
    assert_eq!(app.text("total"), "6.5 30");
    assert_eq!(app.text("describe"), "[id=r1,missing=true,n=42]");
    assert_eq!(app.text("kept"), "[1=kept,2=by C]");
    assert!(
        app.text("failed")
            .ends_with("cwords.count (module `cwords`): argument 1 (text) must be a string"),
        "{}",
        app.text("failed")
    );
    drop(app);
    std::fs::remove_dir_all(dir).unwrap();
}

/// `zgx types` writes an app's modules into its definitions from their
/// manifests, read from the libraries `zgx.json` names: each module's table
/// is typed, so an editor completes its functions and a wrong argument is a
/// type error. A call by name stays untyped.
#[test]
fn an_apps_modules_are_typed_for_its_scripts() {
    use zgx_script::{app_types, check, module_types};
    let dir = std::env::temp_dir().join(format!("zgx-module-types-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    // (A copy: its types are written beside it.)
    let library = dir.join("lib").join(typed_library().file_name().unwrap());
    std::fs::copy(typed_library(), &library).unwrap();
    // A second module: all a tool reads of a library is its manifest, so a
    // file of one stands for it here.
    let few = r#"{"zgx_module":1,"abi":1,"name":"few_more","functions":{"few.now":{"args":[],"returns":{"optional":"number"}}},"actions":{}}"#;
    std::fs::write(
        dir.join("lib/few.bin"),
        zgx_module::abi::manifest_bytes(few),
    )
    .unwrap();
    std::fs::write(
        dir.join("zgx.json"),
        format!(
            r#"{{ "modules": {{ "typed": {:?}, "few_more": "lib/few.bin", "absent": "modules/absent" }} }}"#,
            format!("lib/{}", library.file_name().unwrap().to_string_lossy())
        ),
    )
    .unwrap();
    let good = r#"--!strict
local typed = ui.module("typed")
local words: number = typed.count_words("a b")
local joined: string = typed.join("a", "b")
local found = typed.find({ id = "x", limit = 1 })
local id: string? = if found then found.id else nil
local tags: { string } = if found then found.tags else {}
local now: number? = ui.module("few_more").now()
local by_name: any = ui.call_sync("typed.count_words", "x")
ui.invoke("typed.keep", { any = "value" })
return ui.column {
	ui.text { `{words} {joined} {id} {#tags} {now} {by_name}` },
	ui.button { "Keep", on_click = ui.action("typed.keep", 1) },
}
"#;
    std::fs::write(dir.join("app.luau"), good).unwrap();

    let (modules, untyped) = module_types::of_app(&dir);
    assert_eq!(modules.len(), 2);
    assert_eq!(untyped.len(), 1, "{untyped:?}");
    assert!(untyped[0].starts_with("module `absent`: no built library of it under"));

    assert!(app_types::write_types(&dir).unwrap());
    let written = std::fs::read_to_string(app_types::types_file(&dir)).unwrap();
    for line in [
        "export type TypedQuery = { id: string, limit: number? }",
        "export type TypedFound = { id: string, n: number, tags: { string } }",
        "export type TypedModule = {",
        "\t-- How many words `text` has.\n\tcount_words: (text: string) -> number,",
        "\tjoin: (a: string, b: string, sep: string?) -> string,",
        "\tfind: (query: TypedQuery) -> TypedFound?,",
        "\tsum: (numbers: { number }) -> number,",
        "\tdearest: (prices: { [string]: number }) -> string?,",
        "\tnothing: () -> (),",
        "\tdoubled: (n: number) -> number,",
        "\tany: (value: any) -> any,",
        "export type FewMoreModule = {\n\tnow: () -> number?,\n}",
        // (Modules in `zgx.json`'s order of names.)
        "\tmodule: ((name: \"few_more\") -> FewMoreModule) & ((name: \"typed\") -> TypedModule) & ((name: string) -> any),",
        // By name, untyped as ever.
        "\tcall_sync: (name: string, arg: unknown?) -> any,",
        "\taction: (name: string, arg: unknown?) -> Action,",
        "\tinvoke: (name: string, arg: unknown?) -> (),",
    ] {
        assert!(written.contains(line), "no {line:?} in the app's types");
    }
    let manifests: Vec<String> = modules
        .iter()
        .map(|(_, manifest)| manifest.clone())
        .collect();
    assert_eq!(module_types::notes(&manifests), Vec::<String>::new());
    // Unchanged, it isn't written again; and the module's own types are
    // beside its library.
    assert!(!app_types::write_types(&dir).unwrap());
    let own = std::fs::read_to_string(dir.join("lib/typed.d.luau")).unwrap();
    assert!(own.contains("export type TypedModule = {") && !own.contains("declare ui"));

    // Checked with them (where luau-lsp is installed): the script above is
    // right, and each wrong call is an error at its line.
    if check::find_luau_lsp().is_some() {
        let checked = |script: &str| {
            std::fs::write(dir.join("app.luau"), script).unwrap();
            let report =
                check::check(std::slice::from_ref(&dir), &check::Options::default()).unwrap();
            let errors: Vec<_> = report
                .diagnostics
                .iter()
                .filter(|d| d.severity == check::Severity::Error)
                .collect();
            for error in &errors {
                eprintln!("{error}");
            }
            errors.iter().map(|d| d.line).collect::<Vec<_>>()
        };
        assert_eq!(checked(good), Vec::<u32>::new());
        let bad = r#"--!strict
local typed = ui.module("typed")
local a = typed.count_words(5)
local b = typed.nope("x")
local c = typed.join("a")
local d: string = typed.count_words("x")
local e = typed.find({ id = 7 })
local f: string = ui.module("few_more").now()
return ui.text { `{a} {b} {c} {d} {e} {f}` }
"#;
        assert_eq!(checked(bad), [3, 4, 5, 6, 7, 8]);
    } else {
        eprintln!("skipped the check: luau-lsp is not installed");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// A module that won't load is refused whole, and `ui.module` says why,
/// where it would otherwise only say there is none.
#[test]
fn a_refused_module_is_named_with_why() {
    let dir = std::env::temp_dir().join(format!("zgx-module-refused-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let library = dir.join("libwords.so");
    // A manifest for another interface, in a file that is no library.
    let mut file = vec![0u8; 64];
    file.extend(zgx_module::abi::manifest_bytes(
        r#"{"zgx_module":1,"abi":2,"name":"words"}"#,
    ));
    std::fs::write(&library, file).unwrap();
    let mut why = String::new();
    let mut app = TestApp::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"return ui.text { id = "out", (string.gsub(tostring(select(2, pcall(ui.module, "words"))), "[{}]", "")) }"#,
        );
        // SAFETY: Not a library: refused before anything is loaded.
        why = unsafe { host.load_module(&library) }.unwrap_err();
        host
    });
    assert!(
        why.ends_with("its manifest is for another module interface than this runtime's (1)"),
        "{why}"
    );
    // (Its first line: the traceback follows.)
    let out = app.text("out");
    let first = out.lines().next().unwrap_or("");
    assert!(
        first.ends_with(&format!(
            "module `words` didn't load: {}: its manifest is for another module interface than this runtime's (1)",
            library.display()
        )),
        "{out}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// A module's key covers its crate's files, and only those: a change to one
/// makes another key, saving it as it was makes the first again, and its
/// build output (`target/`) and hidden files make none.
#[test]
fn a_modules_key_follows_its_sources() {
    use zgx_script::module_build::{Profile, key, sources};
    let dir = std::env::temp_dir().join(format!("zgx-module-key-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("target/debug")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"k\"\n").unwrap();
    std::fs::write(dir.join("src/lib.rs"), "fn a() {}\n").unwrap();
    let target = "x86_64-unknown-linux-gnu";
    let first = key(&dir, Profile::Dev, target).unwrap();
    assert_eq!(first.len(), 32);
    assert_eq!(key(&dir, Profile::Dev, target).unwrap(), first);
    // Another profile, another target: other keys.
    assert_ne!(key(&dir, Profile::Release, target).unwrap(), first);
    assert_ne!(
        key(&dir, Profile::Dev, "aarch64-apple-darwin").unwrap(),
        first
    );
    // What isn't a source makes none.
    std::fs::write(dir.join("target/debug/libk.so"), "built").unwrap();
    std::fs::write(dir.join(".editor-swap"), "x").unwrap();
    assert_eq!(key(&dir, Profile::Dev, target).unwrap(), first);
    assert_eq!(
        sources(&dir),
        [dir.join("Cargo.toml"), dir.join("src/lib.rs")]
    );
    // A source's change does, and undoing it is the first key again.
    std::fs::write(dir.join("src/lib.rs"), "fn b() {}\n").unwrap();
    assert_ne!(key(&dir, Profile::Dev, target).unwrap(), first);
    std::fs::write(dir.join("src/lib.rs"), "fn a() {}\n").unwrap();
    assert_eq!(key(&dir, Profile::Dev, target).unwrap(), first);
    // So does a new file, moved or not.
    std::fs::write(dir.join("src/more.rs"), "").unwrap();
    assert_ne!(key(&dir, Profile::Dev, target).unwrap(), first);
    std::fs::remove_dir_all(dir).unwrap();
}

/// `zgx build`'s bundle: its scripts as bytecode, run from its folder as
/// they are asked for; checked against its manifest when opened, by a hash
/// the first time and by its files' stamps after, and refused once a
/// script changes.
#[test]
fn a_bundle_runs_its_bytecode_and_is_checked_once() {
    use zgx_script::app_bundle::{Options, open_with, write};
    let base = std::env::temp_dir().join(format!("zgx-bundle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let app = base.join("app");
    std::fs::create_dir_all(app.join("parts")).unwrap();
    std::fs::write(
        app.join("app.luau"),
        "local part = require(\"./parts/part\")\nreturn ui.text { id = \"out\", part.say() }\n",
    )
    .unwrap();
    std::fs::write(
        app.join("parts/part.luau"),
        "return { say = function() return \"from bytecode\" end }\n",
    )
    .unwrap();
    std::fs::write(
        app.join("zgx.json"),
        r#"{ "app_id": "bundled", "version": "1.2.3" }"#,
    )
    .unwrap();
    // (A cache of its own: what a check remembers.)
    let cache = base.join("cache");
    let open = |dir: &std::path::Path| open_with(dir, &cache);
    let out = base.join("dist");
    let written = write(
        &app,
        &Options {
            out: Some(out.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!((written.files, written.scripts), (3, 2));
    // Its scripts are bytecode: Luau's first byte is its version, never `l`.
    let compiled = std::fs::read(out.join("ui/app.luau")).unwrap();
    assert_ne!(compiled[0], b'l');
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["app_id"], "bundled");
    assert_eq!(manifest["version"], "1.2.3");
    assert_eq!(manifest["ui"]["files"], 3);

    let first = open(&out).unwrap();
    assert!(first.hashed, "the first open hashes it");
    let again = open(&out).unwrap();
    assert!(!again.hashed, "after, its stamps say it is the one checked");
    assert_eq!(again.app_id, "bundled");

    // It runs from its folder, as bytecode.
    let ui = again.ui.clone();
    let mut run = TestApp::new(move |runtime| {
        ScriptHost::from_files(
            runtime,
            zgx_script::ScriptFiles::Folder(ui),
            "app.luau",
            zgx_script::Code::VerifiedBytecode,
        )
    });
    assert_eq!(run.error(), None);
    assert_eq!(run.text("out"), "from bytecode");
    drop(run);

    // A script changed is refused; written again, the bundle is checked anew.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let mut tampered = std::fs::read(out.join("ui/parts/part.luau")).unwrap();
    tampered.push(0);
    std::fs::write(out.join("ui/parts/part.luau"), tampered).unwrap();
    let refused = open(&out).unwrap_err();
    assert!(
        refused.ends_with(
            "its scripts aren't those its manifest names (ui/ changed since it was built)"
        ),
        "{refused}"
    );
    // A folder that is no bundle isn't replaced by one.
    std::fs::create_dir_all(base.join("other")).unwrap();
    std::fs::write(base.join("other/keep.txt"), "mine").unwrap();
    let kept = write(
        &app,
        &Options {
            out: Some(base.join("other")),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        kept.ends_with("not a bundle, and not empty: name another place (--out)"),
        "{kept}"
    );
    assert!(base.join("other/keep.txt").is_file());
    write(
        &app,
        &Options {
            out: Some(out.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(open(&out).unwrap().hashed);
    std::fs::remove_dir_all(base).unwrap();
}
