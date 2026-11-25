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
        ("zgx_api.host_retain", offset_of!(Api, host_retain)),
        ("zgx_api.host_release", offset_of!(Api, host_release)),
        ("zgx_api.post", offset_of!(Api, post)),
        ("zgx_api.after", offset_of!(Api, after)),
        ("zgx_api.timer_release", offset_of!(Api, timer_release)),
        ("zgx_api.state", offset_of!(Api, state)),
        ("zgx_api.shared", offset_of!(Api, shared)),
        ("zgx_api.shared_get", offset_of!(Api, shared_get)),
        ("zgx_api.shared_set", offset_of!(Api, shared_set)),
        ("zgx_api.shared_append", offset_of!(Api, shared_append)),
        ("zgx_api.shared_observe", offset_of!(Api, shared_observe)),
        (
            "zgx_api.shared_unobserve",
            offset_of!(Api, shared_unobserve),
        ),
        ("zgx_api.shared_release", offset_of!(Api, shared_release)),
        ("zgx_api.settle", offset_of!(Api, settle)),
        ("zgx_api.promise_release", offset_of!(Api, promise_release)),
        ("zgx_api.promise_host", offset_of!(Api, promise_host)),
        ("zgx_api.shared_upsert", offset_of!(Api, shared_upsert)),
        ("zgx_api.shared_remove", offset_of!(Api, shared_remove)),
        ("zgx_api.image_buffer", offset_of!(Api, image_buffer)),
        ("zgx_api.image_set", offset_of!(Api, image_set)),
        ("zgx_api.image_drop", offset_of!(Api, image_drop)),
        ("zgx_api.image_remove", offset_of!(Api, image_remove)),
        (
            "zgx_api.shared_upsert_at",
            offset_of!(Api, shared_upsert_at),
        ),
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
        ("zgx_registrar.host_ref", offset_of!(Registrar, host_ref)),
        (
            "zgx_registrar.async_action",
            offset_of!(Registrar, async_action),
        ),
        (
            "zgx_registrar.highlighter",
            offset_of!(Registrar, highlighter),
        ),
    ];
    for (name, rust) in expected {
        assert_eq!(number(name), rust, "{name}");
    }
    // Every line the probe prints is held to something here: a field added
    // to the header is added to this test.
    assert_eq!(c.len(), expected.len() + 4, "{printed}");
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
        format!(
            "{} {} {} {}",
            abi::DONE,
            abi::FAILED,
            abi::REFUSED_ABI,
            abi::SETUP_PANICKED
        )
    );
    assert_eq!(
        c["highlight"],
        format!("{} {}", abi::HIGHLIGHT_FAILED, abi::HIGHLIGHT_STATES)
    );
    assert_eq!(
        c["posted"],
        format!("{} {} {}", abi::POSTED, abi::GONE, abi::HOST_GONE)
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
    let json = r#"{"zgx_module":1,"abi":1,"name":"cwords","version":"1.0.0","records":{"Row":{"id":"string","n":{"optional":"number"}}},"functions":{"cwords.count":{"args":[["text","string"]],"returns":"number"},"cwords.total":{"args":[["numbers",{"list":"number"}],["scale",{"optional":"number"}]],"returns":"number"},"cwords.describe":{"args":[["row",{"record":"Row"}]],"returns":{"map":"any"}},"cwords.kept":{"args":[],"returns":"any"},"cwords.dot":{"args":[["width","number"],["height","number"],["len","number"]],"returns":"string"},"cwords.dot_off_thread":{"args":[],"returns":"string"}},"actions":{"cwords.keep":{"args":[["value","any"]]}}}"#;
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
    compiled("words.c", &library, &["-shared", "-fPIC", "-pthread"]);
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
        local short = select(2, pcall(words.dot, 2, 2, 15))
        return ui.column {
            ui.text { id = "count", show(words.count("a module  in C")) },
            ui.text { id = "name", show(ui.call_sync("cwords.count", "by name")) },
            ui.text { id = "total", `{words.total({ 1, 2, 3.5 })} {words.total({ 1, 2 }, 10)}` },
            ui.text { id = "describe", show(words.describe({ id = "r1", n = 41, zzz = true })) },
            ui.text { id = "kept", show(words.kept()) },
            ui.text { id = "failed", (string.gsub(string.match(tostring(failed), "[^\n]*"), "[{}]", "")) },
            ui.text { id = "short", (string.gsub(string.match(tostring(short), "[^\n]*"), "[{}]", "")) },
            ui.text { id = "dot", words.dot(2, 2, 16) },
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
    // An image by name, its bytes the module's: one of another length is
    // refused, saying what it should be.
    assert_eq!(app.text("dot"), "set");
    let dot = app.host.hosted_image("cwords.dot").expect("set");
    assert_eq!((dot.width(), dot.height()), (2, 2));
    assert!(dot.pixels().iter().all(|b| *b == 0x7f));
    assert!(
        app.text("short")
            .ends_with("`cwords.dot` is 2×2, so 16 bytes of RGBA8 (a row 8 bytes), not 15"),
        "{}",
        app.text("short")
    );
    // Set from another thread: refused, `out` left nil, and nothing changes.
    let off = app
        .host
        .registered_function("cwords.dot_off_thread")
        .unwrap()(Value::Nil)
    .unwrap();
    assert!(
        matches!(&off, Value::Str(s) if &**s == "refused"),
        "{off:?}"
    );
    let still = app.host.hosted_image("cwords.dot").expect("still there");
    assert!(std::sync::Arc::ptr_eq(&still, &dot), "the same image");
    drop((still, dot));
    // Set once its host is gone: refused, saying so, the buffer freed and
    // nothing to crash.
    drop(app);
    // SAFETY: The library this test built and loaded, still loaded; its
    // own exported function, by the type it is written with.
    unsafe {
        let library = libloading::Library::new(&library).unwrap();
        let past: libloading::Symbol<unsafe extern "C" fn() -> u32> =
            library.get(b"cwords_dot_past_host").unwrap();
        assert_eq!(past(), 1, "refused: its host is gone");
    }
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
/// they are asked for, each checked against its manifest's hash as it is
/// read: one changed, or one its manifest doesn't name, is refused.
#[test]
fn a_bundle_runs_its_bytecode_and_checks_each_file_as_it_is_read() {
    use zgx_script::app_bundle::{Options, open, write};
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
    let manifest_text = std::fs::read(out.join("manifest.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_text).unwrap();
    assert_eq!(manifest["app_id"], "bundled");
    assert_eq!(manifest["version"], "1.2.3");
    assert_eq!(manifest["ui"]["files"], 3);
    let mut named: Vec<&String> = manifest["ui"]["sha256"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    named.sort();
    assert_eq!(named, ["app.luau", "parts/part.luau", "zgx.json"]);

    let opened = open(&out).unwrap();
    assert_eq!(opened.app_id, "bundled");
    // It runs from its folder, as bytecode.
    let run_it = |opened: &zgx_script::app_bundle::Opened| {
        let files = opened.files().unwrap();
        TestApp::try_new(move |runtime| {
            ScriptHost::from_files(
                runtime,
                files.clone(),
                "app.luau",
                zgx_script::Code::VerifiedBytecode,
            )
        })
    };
    let mut run = run_it(&opened).unwrap();
    assert_eq!(run.error(), None);
    assert_eq!(run.text("out"), "from bytecode");
    drop(run);

    // A script changed after the bundle was opened is refused as it is
    // read: one byte, the length kept.
    let mut tampered = std::fs::read(out.join("ui/parts/part.luau")).unwrap();
    *tampered.last_mut().unwrap() ^= 1;
    std::fs::write(out.join("ui/parts/part.luau"), &tampered).unwrap();
    let refused = run_it(&opened).err().expect("refused");
    assert!(
        refused.contains("isn't the file its bundle's manifest names (changed since it was built)"),
        "{refused}"
    );
    // A file its manifest doesn't name isn't there, whatever is on the disk.
    *tampered.last_mut().unwrap() ^= 1;
    std::fs::write(out.join("ui/parts/part.luau"), &tampered).unwrap();
    let mut unnamed = manifest.clone();
    unnamed["ui"]["sha256"]
        .as_object_mut()
        .unwrap()
        .remove("parts/part.luau");
    std::fs::write(out.join("manifest.json"), unnamed.to_string()).unwrap();
    let refused = run_it(&open(&out).unwrap()).err().expect("refused");
    assert!(refused.contains("not in the app's bundle"), "{refused}");
    std::fs::write(out.join("manifest.json"), &manifest_text).unwrap();
    assert_eq!(run_it(&open(&out).unwrap()).unwrap().error(), None);
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
    std::fs::remove_dir_all(base).unwrap();
}

/// A bundle's scripts read and checked ahead, on workers: a load takes the
/// bytes a worker checked (a script changed on the disk after that still
/// runs as checked), what no load took goes as the first load ends, and a
/// script that didn't check out ahead is refused as it is read.
#[test]
fn a_bundles_scripts_read_ahead_are_the_bytes_checked() {
    use zgx_script::app_bundle::{Opened, Options, open, write};
    let base = std::env::temp_dir().join(format!("zgx-bundle-ahead-{}", std::process::id()));
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
        "return { say = function() return \"as checked\" end }\n",
    )
    .unwrap();
    std::fs::write(app.join("parts/unused.luau"), "return 1\n").unwrap();
    let out = base.join("dist");
    write(
        &app,
        &Options {
            out: Some(out.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let ready = |opened: &Opened, scripts: usize| {
        let started = std::time::Instant::now();
        while opened.read_ahead_counts() != (scripts, true) {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(10),
                "{:?}",
                opened.read_ahead_counts()
            );
            std::thread::yield_now();
        }
    };
    let run_it = |opened: &Opened| {
        let files = opened.files().unwrap();
        TestApp::try_new(move |runtime| {
            ScriptHost::from_files(
                runtime,
                files.clone(),
                "app.luau",
                zgx_script::Code::VerifiedBytecode,
            )
        })
    };
    let part = out.join("ui/parts/part.luau");
    let checked = std::fs::read(&part).unwrap();
    let mut changed = checked.clone();
    *changed.last_mut().unwrap() ^= 1;

    let opened = open(&out).unwrap();
    opened.read_ahead();
    ready(&opened, 3);
    // Changed on the disk after its check: the load runs what was checked.
    std::fs::write(&part, &changed).unwrap();
    let mut run = run_it(&opened).unwrap();
    assert_eq!(run.text("out"), "as checked");
    drop(run);
    // What no load took (`unused.luau`) went as the load ended; a load
    // after reads the disk, and the change is refused.
    assert_eq!(opened.read_ahead_counts(), (0, true));
    let refused = run_it(&opened).err().expect("refused");
    assert!(
        refused.contains("isn't the file its bundle's manifest names"),
        "{refused}"
    );

    // Changed before the workers read it: not ready, and refused as read.
    let opened = open(&out).unwrap();
    opened.read_ahead();
    ready(&opened, 2);
    let refused = run_it(&opened).err().expect("refused");
    assert!(
        refused.contains("isn't the file its bundle's manifest names"),
        "{refused}"
    );
    std::fs::remove_dir_all(base).unwrap();
}

/// `zgx build --ui source`'s bundle: its files are the app's, run from its
/// folder as any folder is (an edit runs, bytecode doesn't), with its
/// packages in it and its modules checked as any bundle's are.
#[test]
fn a_bundle_of_source_runs_as_its_folder_and_checks_its_modules() {
    use zgx_script::app_bundle::{FORMAT, Options, SOURCE_FORMAT, Ui, open, write};
    let base = std::env::temp_dir().join(format!("zgx-bundle-source-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let write_file = |path: &std::path::Path, text: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    let app = base.join("app");
    let script = "local part = require(\"./parts/part\")\nlocal kit = require(\"@kit/say\")\nreturn ui.text { id = \"out\", part.say() .. kit }\n";
    write_file(&app.join("app.luau"), script);
    write_file(
        &app.join("parts/part.luau"),
        "return { say = function() return \"from source\" end }\n",
    );
    write_file(&base.join("shared/kit/say.luau"), "return \" and kit\"\n");
    write_file(&app.join("words.so"), "a module's library");
    write_file(
        &app.join("zgx.json"),
        r#"{ "app_id": "sourced", "version": "1.2.3", "title": "Sourced",
             "packages": { "kit": "../shared/kit" }, "modules": { "words": "words.so" } }"#,
    );
    let made = |ui: Ui| {
        let out = base.join("dist");
        let options = Options {
            out: Some(out.clone()),
            ui,
            ..Default::default()
        };
        let written = write(&app, &options).unwrap();
        // (The licence in both.)
        let every = if ui == Ui::Source { 2 } else { 1 };
        assert_eq!((written.files, written.scripts), (4 + every, 3));
        out
    };
    // Files no host reads, which source keeps (bytecode the licence alone),
    // and what isn't the app's, which neither does.
    write_file(&app.join("assets/LICENSE.symbols"), "a licence");
    write_file(&app.join("templates/.base.json"), "{}");
    write_file(&app.join(".zgx/types.d.luau"), "");
    write_file(&app.join(".DS_Store"), "");
    // A key or a password left in the folder fails the build, by its name,
    // unless it is allowed: every file there ships.
    let built = |allow: &[&str]| {
        write(
            &app,
            &Options {
                out: Some(base.join("secrets")),
                ui: Ui::Source,
                allow: allow.iter().map(|&a| a.to_owned()).collect(),
                ..Default::default()
            },
        )
    };
    for name in [
        ".env",
        ".env.local",
        ".npmrc",
        ".netrc",
        "certs/Server.PEM",
        "site.key",
        "app.min.p12",
        "cert.pfx",
        "id_ed25519",
        "id_rsa.bak",
    ] {
        write_file(&app.join(name), "secret");
        let refused = built(&[]).unwrap_err();
        assert!(
            refused.contains("looks like a secret") && refused.contains(name),
            "{refused}"
        );
        assert!(
            built(&["other.key"]).is_err(),
            "another allowed, not {name}"
        );
        built(&[name]).unwrap();
        assert!(base.join("secrets/ui").join(name).is_file(), "{name}");
        std::fs::remove_file(app.join(name)).unwrap();
    }
    // So does a folder of such a name, whose files are each looked at too.
    for name in [".env", "id_rsa"] {
        write_file(&app.join(name).join("deep/value.txt"), "secret");
        let refused = built(&[]).unwrap_err();
        assert!(refused.contains(&format!("--allow {name}`")), "{refused}");
        built(&[name]).unwrap();
        write_file(&app.join(name).join("deep/a.pem"), "secret");
        let refused = built(&[name]).unwrap_err();
        assert!(
            refused.contains(&format!("--allow {name}/deep/a.pem`")),
            "{refused}"
        );
        std::fs::remove_dir_all(app.join(name)).unwrap();
    }
    // A package's is allowed as the error says it.
    let in_package = base.join("shared/kit/certs/kit.pem");
    write_file(&in_package, "secret");
    let refused = built(&[]).unwrap_err();
    let said = refused.split("--allow ").nth(1).unwrap();
    let said = said.split('`').next().unwrap();
    assert_eq!(said, ".packages/kit/certs/kit.pem", "{refused}");
    built(&[said]).unwrap();
    assert!(base.join("secrets/ui").join(said).is_file());
    assert!(
        built(&["certs/kit.pem"]).is_err(),
        "not as the package has it"
    );
    std::fs::remove_file(&in_package).unwrap();
    // A name for nothing that ships fails, as does one in a bytecode build.
    for nothing in ["site.kye", "certs", ".zgx/types.d.luau", said] {
        let refused = built(&[nothing]).unwrap_err();
        assert!(
            refused.contains(&format!("--allow {nothing}: no such file")),
            "{refused}"
        );
    }
    let bytecode = Options {
        out: Some(base.join("secrets")),
        allow: vec!["app.luau".into()],
        ..Default::default()
    };
    assert!(write(&app, &bytecode).unwrap_err().contains("--ui source"));
    let fine = [
        "id_card.svg",
        "id_rsa_guide.json",
        "keys.json",
        "key",
        ".env.example",
    ];
    for name in fine {
        write_file(&app.join(name), "{}");
    }
    built(&[]).unwrap();
    for name in fine {
        assert!(base.join("secrets/ui").join(name).is_file(), "{name}");
        std::fs::remove_file(app.join(name)).unwrap();
    }
    // A link out of the app's folder isn't followed into the bundle.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(base.join("shared"), app.join("linked")).unwrap();
        let refused = write(
            &app,
            &Options {
                out: Some(base.join("linked")),
                ui: Ui::Source,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(refused.contains("out of the app's"), "{refused}");
        std::fs::remove_file(app.join("linked")).unwrap();
    }
    // Bytecode is the default, and its format is as it was.
    assert_eq!(Options::default().ui, Ui::Bytecode);
    let manifest = |out: &std::path::Path| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(out.join("manifest.json")).unwrap()).unwrap()
    };
    let dist = made(Ui::Bytecode);
    let checked = manifest(&dist);
    // (And is still opened, by the runtime that opens both.)
    let bytecode = open(&dist).unwrap();
    assert_eq!(bytecode.kind(), Ui::Bytecode);
    assert!(bytecode.files().is_some());
    drop(bytecode);
    assert_eq!((checked["zgx_bundle"].as_u64(), FORMAT), (Some(2), 2));

    let out = made(Ui::Source);
    let source = manifest(&out);
    assert_eq!(
        (source["zgx_bundle"].as_u64(), SOURCE_FORMAT),
        (Some(3), 3),
        "a format of its own: a runtime that reads only 2 refuses it"
    );
    assert_eq!(source["ui"], serde_json::json!({ "source": true }));
    assert_eq!(source["modules"], checked["modules"]);
    let opened = open(&out).unwrap();
    assert_eq!(opened.kind(), Ui::Source);
    assert!(opened.files().is_none());
    assert_eq!(
        (opened.app_id.as_str(), opened.version.as_str()),
        ("sourced", "1.2.3")
    );
    assert_eq!(opened.modules[0].0, "words");
    assert_eq!(
        std::fs::read_to_string(out.join("ui/app.luau")).unwrap(),
        script
    );
    assert_eq!(opened.file("app.luau").unwrap().unwrap(), script.as_bytes());
    for kept in ["assets/LICENSE.symbols", "templates/.base.json"] {
        assert!(out.join("ui").join(kept).is_file(), "{kept}");
    }
    // (A module's library is the manifest's, in modules/.)
    for left in [".zgx", ".DS_Store", "words.so"] {
        assert!(!out.join("ui").join(left).exists(), "{left}");
    }
    assert_eq!(opened.file("none.luau").unwrap(), None);

    // It runs from its folder alone (the app's own, and its package, gone),
    // and an edit there runs.
    std::fs::rename(&app, base.join("app-away")).unwrap();
    std::fs::rename(base.join("shared"), base.join("shared-away")).unwrap();
    let entry = opened.ui.join(&opened.entry);
    let run_it = || {
        let entry = entry.clone();
        TestApp::try_new(move |runtime| {
            let dir = entry.parent().unwrap();
            let config = zgx_script::app_config::read(dir).unwrap();
            config.apply(ScriptHost::new(runtime, entry.clone()))
        })
    };
    assert_eq!(run_it().unwrap().text("out"), "from source and kit");
    let part = out.join("ui/parts/part.luau");
    std::fs::write(&part, "return { say = function() return \"edited\" end }\n").unwrap();
    assert_eq!(run_it().unwrap().text("out"), "edited and kit");
    // Bytecode there is no source, and doesn't run.
    let bytecode = zgx_script::build::compile(b"return { say = function() return 'x' end }");
    std::fs::write(&part, bytecode.unwrap()).unwrap();
    assert!(run_it().is_err(), "bytecode run from a folder");
    // Nothing out of its `ui/` is read as its scripts: not by `..`, nor by a
    // link in it.
    std::fs::write(
        base.join("out.luau"),
        "return { say = function() return \"out\" end }\n",
    )
    .unwrap();
    assert!(opened.file("../manifest.json").is_err());
    let escape = |how: &str| {
        std::fs::write(&part, format!("return require(\"{how}\")\n")).unwrap();
        let refused = run_it().err().expect("refused");
        assert!(
            refused.contains("modules come from the script's directory"),
            "{refused}"
        );
    };
    escape("../../../out");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(base.join("out.luau"), out.join("ui/link.luau")).unwrap();
        escape("../link");
        let refused = opened.file("link.luau").unwrap_err();
        assert!(refused.contains("outside its bundle's ui/"), "{refused}");
    }
    // A manifest whose first script is out of it, or that says both kinds, or
    // neither, or another format's, is refused.
    let path = out.join("manifest.json");
    for (format, ui) in [
        (3, serde_json::json!({ "source": true, "sha256": {} })),
        (3, serde_json::json!({})),
        (3, serde_json::json!({ "source": false })),
        (3, checked["ui"].clone()),
        (2, serde_json::json!({ "source": true, "sha256": {} })),
        (2, serde_json::json!({})),
        (2, source["ui"].clone()),
    ] {
        let mut wrong = source.clone();
        (wrong["zgx_bundle"], wrong["ui"]) = (format.into(), ui);
        std::fs::write(&path, wrong.to_string()).unwrap();
        let refused = open(&out).unwrap_err();
        assert!(
            refused.contains("its `ui` isn't what a bundle of its format has"),
            "{refused}"
        );
    }
    let mut entries = vec!["../../out.luau"];
    entries.extend(cfg!(unix).then_some("link.luau"));
    for entry in entries {
        let mut wrong = source.clone();
        wrong["entry"] = entry.into();
        std::fs::write(&path, wrong.to_string()).unwrap();
        let refused = open(&out).unwrap_err();
        assert!(refused.contains("outside its bundle's ui/"), "{refused}");
    }
    std::fs::write(&path, source.to_string()).unwrap();
    // A module's library changed since the build is refused, as any bundle's.
    let library = opened.modules[0].1.clone();
    std::fs::write(&library, "another library").unwrap();
    let refused = open(&out).unwrap_err();
    assert!(
        refused.contains("isn't the library its manifest names"),
        "{refused}"
    );
    std::fs::remove_dir_all(base).unwrap();
}

/// A bundle of bytecode keeps the licences beside the files it ships, and
/// says which of the app's files it left out; one of source leaves out none.
#[test]
fn a_bundle_keeps_licences_and_says_what_it_left_out() {
    use zgx_script::app_bundle::{Options, Ui, write};
    let base = std::env::temp_dir().join(format!("zgx-bundle-left-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let write_file = |path: &std::path::Path| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "{}").unwrap();
    };
    let app = base.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("app.luau"), "return ui.text 'x'").unwrap();
    std::fs::write(
        app.join("zgx.json"),
        r#"{ "packages": { "kit": "../kit" } }"#,
    )
    .unwrap();
    let licences = [
        "LICENSE",
        "icons/LICENSE.symbols",
        "icons/Licence-MIT.md",
        "fonts/COPYING",
        "fonts/NOTICES.txt",
        "LICENSES/MIT.txt",
    ];
    let left = [
        ".base.json",
        ".cache/",
        "noticeboard.txt",
        "licensed.txt",
        "sounds/click.wav",
        "templates/v2/.base.json",
    ];
    for name in licences.iter().chain(&left) {
        let name = name.replace(".cache/", ".cache/deep/file.json");
        write_file(&app.join(name));
    }
    for name in ["LICENSE", "readme.txt", ".hidden"] {
        write_file(&base.join("kit").join(name));
    }
    // Not the app's: not told.
    for name in [".git/config", ".zgx/types.d.luau", ".DS_Store"] {
        write_file(&app.join(name));
    }
    let made = |ui: Ui| {
        let out = base.join("dist");
        let options = Options {
            out: Some(out.clone()),
            ui,
            ..Default::default()
        };
        (out, write(&app, &options).unwrap().left_out)
    };
    let (out, told) = made(Ui::Bytecode);
    for name in licences.iter().chain(&[".packages/kit/LICENSE"]) {
        assert!(out.join("ui").join(name).is_file(), "{name}");
    }
    let mut expected: Vec<String> = left.iter().map(|&name| name.to_owned()).collect();
    expected.extend([".packages/kit/.hidden", ".packages/kit/readme.txt"].map(str::to_owned));
    expected.sort();
    assert_eq!(told, expected);
    for name in &told {
        assert!(!out.join("ui").join(name).exists(), "{name}");
    }
    let (out, told) = made(Ui::Source);
    assert_eq!(told, Vec::<String>::new());
    for name in &expected {
        assert!(out.join("ui").join(name).exists(), "{name}");
    }
    std::fs::remove_dir_all(base).unwrap();
}

/// A package a bundled app lends its plugins (a design system in its own
/// folder) is the bundle's bytecode: a plugin, whose own files are source
/// on the disk, requires its modules as the app does.
#[test]
fn a_bundles_lent_package_is_its_bytecode_for_plugins_too() {
    use zgx_script::app_bundle::{Options, open, write};
    use zgx_script::{Capability, Plugin};
    let base = std::env::temp_dir().join(format!("zgx-bundle-lent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let write_file = |path: &std::path::Path, text: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    let app = base.join("app");
    write_file(
        &app.join("app.luau"),
        "local badge = require(\"@ds/badge\")\nreturn ui.column { id = \"app\", ui.text { id = \"mine\", badge(\"app\") } }\n",
    );
    write_file(
        &app.join("ds/badge.luau"),
        "return function(label) return label .. \" calm\" end\n",
    );
    let out = base.join("dist");
    write(
        &app,
        &Options {
            out: Some(out.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let opened = open(&out).unwrap();
    assert_ne!(
        std::fs::read(opened.ui.join("ds/badge.luau")).unwrap()[0],
        b'r',
        "bytecode"
    );
    let plugins = base.join("plugins");
    write_file(
        &plugins.join("p.luau"),
        "local badge = require(\"@ds/badge\")\nui.contribute { target = \"#mine\", after = ui.text { id = \"theirs\", badge(\"plugin\") } }\n",
    );
    let (ui, files) = (opened.ui.clone(), opened.files().unwrap());
    let mut run = TestApp::new(move |runtime| {
        let host = ScriptHost::from_files(
            runtime,
            files.clone(),
            "app.luau",
            zgx_script::Code::VerifiedBytecode,
        )
        .package("ds", ui.join("ds"))
        .grant_all_plugins(Capability::Package("ds".into()));
        host.set_plugins(vec![Plugin {
            name: "p".into(),
            entry: plugins.join("p.luau"),
            grants: Vec::new(),
        }]);
        host
    });
    run.frames(1);
    assert_eq!(run.error(), None);
    assert_eq!(run.text("mine"), "app calm");
    assert_eq!(run.text("theirs"), "plugin calm");
    drop(run);
    std::fs::remove_dir_all(base).unwrap();
}

/// A module's reach into its host (stage 2): its threads post work to the
/// interface's thread, and it keeps timers on zgx's clock. Linked in, as a
/// module the test writes.
mod reach {
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        thread::ThreadId,
        time::Duration,
    };
    use zgx_module::{Host, ImageBuffer, Observer, Ref, State, Timer, Ui, Value};

    /// What the module saw, on the interface's thread.
    #[derive(Default)]
    pub struct Seen {
        pub thread: Option<ThreadId>,
        /// Each job's number, in the order the jobs ran, and whether each
        /// ran on the interface's thread.
        pub jobs: Vec<(f64, bool)>,
        /// What each timer's call said, and when (the test's time, by the
        /// frames it was given).
        pub fired: Vec<String>,
        pub timers: Vec<Timer>,
        /// What `query` held each time a script set it.
        pub heard: Vec<String>,
        pub observer: Option<Observer>,
    }
    /// A host handle kept past its host, for a test of a post after it.
    pub static KEPT: Mutex<Option<Ui>> = Mutex::new(None);
    /// A shared state's handle kept past its host (the test's own thread
    /// alone reads it: it isn't `Send`).
    pub struct SharedKept(pub zgx_module::Shared);
    // SAFETY: Only the test's thread, the interface's, takes it out.
    unsafe impl Send for SharedKept {}
    pub static KEPT_SHARED: Mutex<Option<SharedKept>> = Mutex::new(None);
    thread_local! {
        static QUERY: std::cell::RefCell<Option<zgx_module::Shared>> = const { std::cell::RefCell::new(None) };
    }
    fn query_kept() -> zgx_module::Shared {
        QUERY.with_borrow_mut(|q| q.take()).expect("a query kept")
    }
    /// A promise held for `reach.hold`.
    pub static HELD: Mutex<Option<zgx_module::Promise>> = Mutex::new(None);
    /// How many jobs were dropped, run or not.
    pub static DROPPED: AtomicUsize = AtomicUsize::new(0);

    /// Dropped with its job.
    pub struct Counted;
    impl Drop for Counted {
        fn drop(&mut self) {
            DROPPED.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn seen(host: &Host) -> State<Seen> {
        host.state::<Seen>()
    }

    zgx_module::register!("reach", |host: &Host| {
        let state = seen(host);
        QUERY.with_borrow_mut(|q| *q = host.shared("query", Value::from("")));
        state.with(|seen| seen.thread = Some(std::thread::current().id()));
        let ui = host.ui();
        // `n` jobs posted from a thread of the module's own, in order.
        host.function("reach.post", move |n| {
            let (n, ui) = (n.num().unwrap_or(0.), ui.clone());
            std::thread::spawn(move || {
                for i in 0..n as usize {
                    let counted = Counted;
                    ui.post(move |host| {
                        let _counted = counted;
                        seen(host).with(|seen| {
                            let here = seen.thread == Some(std::thread::current().id());
                            seen.jobs.push((i as f64, here));
                        });
                    });
                }
            })
            .join()
            .unwrap();
            Value::NIL
        });
        // Each job's number, then whether all ran on the interface's thread.
        host.function("reach.jobs", move |_| {
            state.with(|seen| {
                Value::list(
                    seen.jobs
                        .iter()
                        .map(|(i, here)| Value::list([Value::from(*i), Value::from(*here)])),
                )
            })
        });
        // A timer once after `{ after = ms }`, one every `{ every = ms }`.
        let kept = host.ui();
        host.function("reach.timers", move |arg| {
            let (after, every) = (
                arg.field("after").and_then(|v| v.num()),
                arg.field("every").and_then(|v| v.num()),
            );
            // (A job is the interface thread's: it makes them there.)
            kept.post(move |host| {
                let mut made = Vec::new();
                if let Some(ms) = after {
                    made.push(host.after(Duration::from_secs_f64(ms / 1e3), |host| {
                        seen(host).with(|seen| seen.fired.push("after".into()));
                    }));
                }
                if let Some(ms) = every {
                    made.push(host.every(Duration::from_secs_f64(ms / 1e3), |host| {
                        seen(host).with(|seen| seen.fired.push("every".into()));
                    }));
                }
                seen(host).with(|seen| seen.timers.extend(made));
            });
            Value::NIL
        });
        // Its timers dropped, stopping them.
        host.function("reach.stop", move |_| {
            let timers = state.with(|seen| std::mem::take(&mut seen.timers));
            drop(timers);
            Value::NIL
        });
        host.function("reach.fired", move |_| {
            state.with(|seen| Value::list(seen.fired.iter().map(|f| Value::from(f.as_str()))))
        });
        // A model published from a thread: `{ n }` rows, set by a job.
        let publisher = host.ui();
        host.function("reach.publish", move |n| {
            let (n, ui) = (n.num().unwrap_or(0.) as usize, publisher.clone());
            std::thread::spawn(move || {
                ui.post(move |host| {
                    let feed = host.shared("feed", Value::NIL).unwrap();
                    feed.set(Value::list((0..n).map(|i| {
                        Value::record([
                            ("id", Value::from(i as f64)),
                            ("title", Value::from("row")),
                        ])
                    })))
                    .unwrap();
                });
            })
            .join()
            .unwrap();
            Value::NIL
        });
        // Rows changed on a thread, posted as a patch: `{ set = { {id,
        // title}... }, gone = { id... } }`.
        let patcher = host.ui();
        host.function("reach.patch", move |patch| {
            let text = |r: Option<Ref<'_>>| r.and_then(|r| r.str()).unwrap_or("").to_owned();
            let set: Vec<(String, String)> = patch
                .field("set")
                .map(|set| {
                    set.items()
                        .map(|r| (text(r.field("id")), text(r.field("title"))))
                        .collect()
                })
                .unwrap_or_default();
            let gone: Vec<String> = patch
                .field("gone")
                .map(|gone| gone.items().map(|id| text(Some(id))).collect())
                .unwrap_or_default();
            let ui = patcher.clone();
            std::thread::spawn(move || {
                ui.post(move |host| {
                    let feed = host.shared("feed", Value::NIL).unwrap();
                    let rows = set.iter().map(|(id, title)| {
                        Value::record([
                            ("id", Value::from(id.as_str())),
                            ("title", Value::from(title.as_str())),
                        ])
                    });
                    feed.upsert("id", rows).unwrap();
                    feed.remove("id", gone.iter().map(String::as_str)).unwrap();
                });
            })
            .join()
            .unwrap();
            Value::NIL
        });
        // A `{ name, w, h, shade }` picture written on a thread, set by a
        // job it posts: what setting it said, and where its bytes were.
        let painter = host.ui();
        host.function("reach.paint", move |arg| {
            let name = arg
                .field("name")
                .and_then(|n| n.str())
                .unwrap_or("")
                .to_owned();
            let num = |k: &str| arg.field(k).and_then(|n| n.num()).unwrap_or(0.);
            let (w, h, shade) = (num("w") as u32, num("h") as u32, num("shade") as u8);
            let ui = painter.clone();
            std::thread::spawn(move || {
                let mut pixels = ImageBuffer::new(w, h).unwrap();
                pixels.pixels().fill(shade);
                *PAINTED.lock().unwrap() = pixels.pixels().as_ptr() as usize;
                ui.post(move |host| {
                    *IMAGE_SAID.lock().unwrap() =
                        Some(host.image(&name, pixels).err().map(|f| f.0));
                });
            })
            .join()
            .unwrap();
            Value::NIL
        });
        // `name`'s image removed, by a job: whether there was one.
        let remover = host.ui();
        host.function("reach.unpaint", move |name| {
            let name = name.str().unwrap_or("").to_owned();
            remover.post(move |host| *UNPAINTED.lock().unwrap() = Some(host.remove_image(&name)));
            Value::NIL
        });
        // Rows placed, from a thread: `{ {id, title, before?, at_end?}... }`;
        // what writing them said (`Some(None)`: written).
        let placer = host.ui();
        host.function("reach.place", move |given| {
            let text = |r: Option<Ref<'_>>| r.and_then(|r| r.str()).map(str::to_owned);
            let rows: Vec<(String, String, Option<String>, bool)> = given
                .items()
                .map(|r| {
                    let at_end = r.field("at_end").and_then(|e| e.bool()).unwrap_or(false);
                    (
                        text(r.field("id")).unwrap_or_default(),
                        text(r.field("title")).unwrap_or_default(),
                        text(r.field("before")),
                        at_end,
                    )
                })
                .collect();
            let ui = placer.clone();
            std::thread::spawn(move || {
                ui.post(move |host| {
                    let feed = host.shared("feed", Value::NIL).unwrap();
                    let placed = rows.iter().map(|(id, title, before, at_end)| {
                        let row = Value::record([
                            ("id", Value::from(id.as_str())),
                            ("title", Value::from(title.as_str())),
                        ]);
                        let place = match (before, at_end) {
                            (Some(key), _) => zgx_module::Place::Before(key),
                            (None, true) => zgx_module::Place::End,
                            (None, false) => zgx_module::Place::Stay,
                        };
                        (row, place)
                    });
                    *PLACED.lock().unwrap() = Some(feed.upsert_at("id", placed).err().map(|f| f.0));
                });
            })
            .join()
            .unwrap();
            Value::NIL
        });
        // A row with no `id`: refused, saying why.
        let unkeyed = host.shared("feed", Value::NIL).unwrap();
        host.function("reach.patch_unkeyed", move |_| {
            let refused = unkeyed.upsert("id", [Value::record([("title", Value::from("x"))])]);
            Value::from(refused.unwrap_err().0.as_str())
        });
        // The same model again: nothing changes.
        let feed = host.shared("feed", Value::NIL).unwrap();
        host.function("reach.republish", move |_| {
            let same = feed.get().get().to_owned();
            Value::from(feed.set(same).unwrap())
        });
        // How many rows the feed holds, read in a binding.
        let counted = host.shared("feed", Value::NIL).unwrap();
        host.function("reach.feed_len", move |_| {
            Value::from(counted.get().get().len() as f64)
        });
        // Text added to `log`, and to the feed (a list: refused).
        let (log, not_text) = (
            host.shared("log", Value::from("")).unwrap(),
            host.shared("feed", Value::NIL).unwrap(),
        );
        host.function("reach.append", move |text| {
            let text = text.str().unwrap_or("");
            let listed = not_text.append(text).err().map(|why| why.0);
            log.append(text).unwrap();
            listed.map_or(Value::NIL, |why| Value::from(why.as_str()))
        });
        // What a script set `query` to, each time.
        let query = host.shared("query", Value::from("")).unwrap();
        let observer = query.observe(|host, now| {
            let now = now.str().unwrap_or("?").to_owned();
            seen(host).with(|seen| seen.heard.push(now));
        });
        state.with(|seen| seen.observer = Some(observer));
        host.function("reach.heard", move |_| {
            state.with(|seen| Value::list(seen.heard.iter().map(|h| Value::from(h.as_str()))))
        });
        // A handle to `feed` kept past the host.
        host.function("reach.keep_shared", move |_| {
            *KEPT_SHARED.lock().unwrap() = Some(SharedKept(query_kept()));
            Value::NIL
        });
        // `n` doubled later, by a thread.
        host.async_action("reach.later", |n, promise| {
            let n = n.num().unwrap_or(0.);
            std::thread::spawn(move || promise.resolve_with(move |_| Ok(Value::from(n * 2.))));
        });
        // Refused later, saying why.
        host.async_action("reach.refuse", |_, promise| {
            let ui = promise.ui();
            ui.post(move |host| promise.reject(host, "has no such row"));
        });
        // Dropped on a thread, unsettled.
        host.async_action("reach.drop_it", |_, promise| {
            std::thread::spawn(move || drop(promise)).join().unwrap();
        });
        // Held on a thread until `reach.release`, then settled: past its
        // host, in the test of one gone.
        host.async_action("reach.hold", |_, promise| {
            *HELD.lock().unwrap() = Some(promise);
        });
        // A handle kept past the host.
        let keep = host.ui();
        host.function("reach.keep", move |_| {
            *KEPT.lock().unwrap() = Some(keep.clone());
            Value::NIL
        });
    });

    /// Where the last picture `reach.paint` wrote was, and what setting
    /// it said (`Some(None)`: set).
    pub static PAINTED: Mutex<usize> = Mutex::new(0);
    pub static IMAGE_SAID: Mutex<Option<Option<String>>> = Mutex::new(None);
    /// What `reach.place` writing its rows said (`Some(None)`: written).
    pub static PLACED: Mutex<Option<Option<String>>> = Mutex::new(None);
    /// Whether `reach.unpaint` removed one.
    pub static UNPAINTED: Mutex<Option<bool>> = Mutex::new(None);

    /// Whether `post` from a thread after the host went was refused, with
    /// its job dropped unrun.
    pub fn post_after(ran: Arc<AtomicBool>) -> bool {
        let ui = KEPT.lock().unwrap().take().expect("a kept handle");
        std::thread::spawn(move || {
            let counted = Counted;
            ui.post(move |_| {
                let _counted = counted;
                ran.store(true, Ordering::SeqCst);
            })
        })
        .join()
        .unwrap()
    }
}

const REACHING: &str = r#"
local feed = ui.shared("feed")
local query = ui.shared("query", "")
local log = ui.shared("log", "")
local said = ui.state("said", "")
return ui.column {
    ui.text { id = "out", "x" },
    ui.text { id = "rows", function() local f = feed:get(); return if f then `{#f} rows` else "none" end },
    ui.text { id = "first", function() local f = feed:get(); return if f and f[1] then f[1].title else "-" end },
    ui.text { id = "len", function() return tostring(ui.call_sync("reach.feed_len")) end },
    ui.text { id = "log", function() return log:get() end },
    ui.text { id = "said", function() return said:get() end },
    ui.row {
    ui.button { id = "ask", on_click = function() query:set("first") end, "ask" },
    ui.button { id = "again", on_click = function() query:set("second") end, "again" },
    ui.button { id = "later", on_click = function() ui.call("reach.later", 21):after(function(v) said:set(`got {v}`) end) end, "later" },
    ui.button { id = "member", on_click = function() ui.module("reach").later(4):after(function(v) said:set(`member {v}`) end) end, "member" },
    },
    ui.row {
    ui.button { id = "refuse", on_click = function() ui.call("reach.refuse"):after(nil, function(e) said:set(e.message) end) end, "refuse" },
    ui.button { id = "drop", on_click = function() ui.call("reach.drop_it"):after(nil, function(e) said:set(e.message) end) end, "drop" },
    ui.button { id = "hold", on_click = function() ui.call("reach.hold"):after(function() said:set("held settled") end) end, "hold" },
    },
}
"#;

/// An app with the `reach` module linked in.
fn reaching() -> TestApp {
    TestApp::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", REACHING);
        // SAFETY: The test's own `register!` entry.
        unsafe { host.enter_module(reach::zgx_module_v1, &reach::zgx_module_manifest) }.unwrap();
        host
    })
}
fn called(app: &TestApp, name: &str, arg: Value) -> Value {
    app.host
        .registered_function(name)
        .unwrap_or_else(|| panic!("no {name}"))(arg)
    .unwrap()
}

/// A module's thread posts jobs to its host: they run on the interface's
/// thread, in the order posted, each freed after.
#[test]
fn a_modules_thread_posts_work_to_the_interfaces_thread() {
    let mut app = reaching();
    let dropped = reach::DROPPED.load(std::sync::atomic::Ordering::SeqCst);
    called(&app, "reach.post", Value::Num(5.));
    assert!(app.wait_until(|app| {
        matches!(called(app, "reach.jobs", Value::Nil), Value::List(l) if l.len() == 5)
    }));
    let Value::List(jobs) = called(&app, "reach.jobs", Value::Nil) else {
        unreachable!()
    };
    let jobs: Vec<String> = jobs.iter().map(|job| format!("{job:?}")).collect();
    let want: Vec<String> = (0..5)
        .map(|i| {
            format!(
                "{:?}",
                Value::List(Slice::from(vec![
                    Value::Num(f64::from(i)),
                    Value::Bool(true)
                ]))
            )
        })
        .collect();
    assert_eq!(jobs, want);
    assert!(reach::DROPPED.load(std::sync::atomic::Ordering::SeqCst) >= dropped + 5);
}

/// A post to a host that is gone is refused, and its job dropped, unrun.
#[test]
fn a_post_after_its_host_is_gone_is_refused() {
    let app = reaching();
    called(&app, "reach.keep", Value::Nil);
    drop(app);
    let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let dropped = reach::DROPPED.load(std::sync::atomic::Ordering::SeqCst);
    assert!(!reach::post_after(ran.clone()), "refused");
    assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
    assert!(
        reach::DROPPED.load(std::sync::atomic::Ordering::SeqCst) > dropped,
        "freed"
    );
}

/// A module's timers keep zgx's clock (here a test's, moved by its frames):
/// one after its delay, once; one every period, until it is dropped.
#[test]
fn a_modules_timers_keep_zgxs_clock() {
    let app = reaching();
    called(
        &app,
        "reach.timers",
        Value::record([("after", Value::Num(100.)), ("every", Value::Num(70.))]),
    );
    app.settle();
    let fired = |app: &TestApp| {
        let Value::List(fired) = called(app, "reach.fired", Value::Nil) else {
            unreachable!()
        };
        fired
            .iter()
            .filter_map(|f| match f {
                Value::Str(f) => Some(f.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert!(fired(&app).is_empty());
    // 8 frames, 133 ms: the one every 70 ms once, then the one after 100.
    // (They're made a frame in at most, by a job: 17 ms.)
    app.frames(8);
    assert_eq!(fired(&app), ["every", "after"]);
    called(&app, "reach.stop", Value::Nil);
    app.frames(12);
    assert_eq!(fired(&app), ["every", "after"], "stopped");
}

/// An app whose module holds a handle to its host asks its window for
/// nothing at rest: no timer, no poll. One post wakes it once.
#[test]
fn a_module_holding_its_host_costs_an_idle_app_nothing() {
    let app = reaching();
    app.settle();
    let at_rest = app.wakes();
    for _ in 0..5 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        app.settle();
    }
    assert_eq!(app.wakes(), at_rest, "no wakes at rest");
    assert_eq!(app.executor.borrow_mut().tick(), 0, "nothing ready");
    called(&app, "reach.post", Value::Num(1.));
    assert_eq!(app.wakes(), at_rest + 1, "one post, one wake");
    app.settle();
    assert!(matches!(called(&app, "reach.jobs", Value::Nil), Value::List(l) if l.len() == 1));
    assert_eq!(app.wakes(), at_rest + 1);
}

/// A module publishes its model in a shared state from a job its thread
/// posts: scripts show it, and a binding that asks the module for it
/// follows it. The same model again changes nothing.
#[test]
fn a_module_publishes_its_model_in_shared_state() {
    let mut app = reaching();
    assert_eq!(app.text("rows"), "none");
    assert_eq!(app.text("len"), "0");
    called(&app, "reach.publish", Value::Num(3.));
    assert!(app.wait_until(|app| app.text("rows") == "3 rows"));
    assert_eq!(app.text("first"), "row");
    assert_eq!(app.text("len"), "3", "followed through the module");
    assert!(matches!(
        called(&app, "reach.republish", Value::Nil),
        Value::Bool(false)
    ));
    // The host sees the same state.
    let Value::List(rows) = app.host.shared("feed", Value::Nil).get() else {
        panic!("a list")
    };
    assert_eq!(rows.len(), 3);
}

/// A module's thread posts only the rows that changed: they replace theirs
/// in the list where they are, new ones follow, removed ones go, and
/// scripts show it. A row with no key is refused, saying why.
#[test]
fn a_module_patches_a_shared_list_by_key() {
    let mut app = reaching();
    let row = |id: &str, title: &str| {
        Value::record([("id", Value::from(id)), ("title", Value::from(title))])
    };
    let patch = |set: Vec<Value>, gone: Vec<&str>| {
        Value::record([
            ("set", Value::List(Slice::from(set))),
            (
                "gone",
                Value::List(Slice::from(
                    gone.into_iter().map(Value::from).collect::<Vec<_>>(),
                )),
            ),
        ])
    };
    called(
        &app,
        "reach.patch",
        patch(
            vec![row("a", "one"), row("b", "two"), row("c", "three")],
            vec![],
        ),
    );
    assert!(app.wait_until(|app| app.text("rows") == "3 rows"));
    let before = app.host.shared("feed", Value::Nil).get();
    called(
        &app,
        "reach.patch",
        patch(vec![row("a", "ONE"), row("d", "four")], vec!["b"]),
    );
    assert!(app.wait_until(|app| app.text("first") == "ONE"));
    assert_eq!(app.text("rows"), "3 rows");
    let after = app.host.shared("feed", Value::Nil).get();
    let titles = |list: &Value| -> Vec<String> {
        list.list()
            .iter()
            .map(|r| match r.field("title") {
                Some(Value::Str(t)) => t.to_string(),
                _ => "-".into(),
            })
            .collect()
    };
    assert_eq!(titles(&after), ["ONE", "three", "four"]);
    let (Value::List(before), Value::List(after)) = (&before, &after) else {
        panic!("lists")
    };
    assert!(
        matches!((&before[2], &after[1]), (Value::Map(b), Value::Map(a)) if Slice::ptr_eq(b, a)),
        "c is the row it was"
    );
    let refused = called(&app, "reach.patch_unkeyed", Value::Nil);
    assert!(
        matches!(&refused, Value::Str(why) if &**why == "row 1 has no string `id`"),
        "{refused:?}"
    );
}

/// The tests that paint, one at a time: they read what the module's
/// statics say.
static PAINTING: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn paint(app: &TestApp, name: &str, w: f64, h: f64, shade: f64) -> Option<String> {
    *reach::IMAGE_SAID.lock().unwrap() = None;
    let arg = Value::record([
        ("name", Value::from(name)),
        ("w", Value::Num(w)),
        ("h", Value::Num(h)),
        ("shade", Value::Num(shade)),
    ]);
    called(app, "reach.paint", arg);
    app.settle();
    reach::IMAGE_SAID
        .lock()
        .unwrap()
        .take()
        .expect("the job ran")
}

/// A picture a module's thread writes shows by name, its bytes the very
/// ones written (not copied); a name not the module's is refused; removed,
/// it goes.
#[test]
fn a_modules_picture_shows_by_name_uncopied() {
    let _one = PAINTING.lock().unwrap_or_else(|e| e.into_inner());
    let app = reaching();
    assert_eq!(paint(&app, "reach.cover", 3., 2., 200.), None);
    let cover = app.host.hosted_image("reach.cover").expect("set");
    assert_eq!((cover.width(), cover.height()), (3, 2));
    assert_eq!(cover.pixels().len(), 24);
    assert!(cover.pixels().iter().all(|b| *b == 200));
    assert_eq!(
        cover.pixels().as_ptr() as usize,
        *reach::PAINTED.lock().unwrap(),
        "not copied"
    );
    let refused = paint(&app, "cover", 1., 1., 0.).expect("refused");
    assert_eq!(
        refused,
        "an image a module sets is named `<module>.<name>`, not `cover`"
    );
    assert!(app.host.hosted_image("cover").is_none());
    let unpainted = |app: &TestApp, name: &str| {
        called(app, "reach.unpaint", Value::from(name));
        app.settle();
        reach::UNPAINTED
            .lock()
            .unwrap()
            .take()
            .expect("the job ran")
    };
    assert!(unpainted(&app, "reach.cover"));
    assert!(app.host.hosted_image("reach.cover").is_none());
    assert!(!unpainted(&app, "reach.cover"), "gone already");
}

/// What a module sets goes with its host; loaded into its host again, the
/// images its last load set go (the host's own stay).
#[test]
fn a_modules_pictures_go_with_its_host_and_its_reload() {
    let _one = PAINTING.lock().unwrap_or_else(|e| e.into_inner());
    let app = reaching();
    assert_eq!(paint(&app, "reach.cover", 2., 2., 9.), None);
    let held = std::sync::Arc::downgrade(&app.host.hosted_image("reach.cover").unwrap());
    app.host.image(
        "art",
        Some(std::sync::Arc::new(
            zgx::image::ImageData::new(1, 1, vec![1; 4]).unwrap(),
        )),
    );
    // SAFETY: The test's own `register!` entry, loaded again.
    unsafe {
        app.host
            .enter_module(reach::zgx_module_v1, &reach::zgx_module_manifest)
    }
    .unwrap();
    assert!(
        app.host.hosted_image("reach.cover").is_none(),
        "its last load's"
    );
    assert!(held.upgrade().is_none(), "freed");
    assert!(app.host.hosted_image("art").is_some(), "the host's own");
    assert_eq!(paint(&app, "reach.cover", 2., 2., 9.), None);
    let held = std::sync::Arc::downgrade(&app.host.hosted_image("reach.cover").unwrap());
    drop(app);
    assert!(held.upgrade().is_none(), "gone with its host");
}

/// A module's thread posts rows with places: one moved to the end, one of
/// the list's and one new placed before it, which go with it; scripts
/// show the list in its new order. One placed before itself is refused, saying so.
#[test]
fn a_module_places_rows_by_key() {
    let mut app = reaching();
    let row = |id: &str, title: &str| {
        Value::record([("id", Value::from(id)), ("title", Value::from(title))])
    };
    let place = |app: &TestApp, rows: Vec<Value>| -> Option<String> {
        *reach::PLACED.lock().unwrap() = None;
        called(app, "reach.place", Value::List(Slice::from(rows)));
        app.settle();
        reach::PLACED.lock().unwrap().take().expect("the job ran")
    };
    let placed = |id: &str, title: &str, place: (&str, Value)| {
        Value::record([
            ("id", Value::from(id)),
            ("title", Value::from(title)),
            (place.0, place.1),
        ])
    };
    assert_eq!(
        place(&app, vec![row("a", "A"), row("b", "B"), row("c", "C")]),
        None
    );
    assert_eq!(
        place(
            &app,
            vec![
                placed("c", "C", ("before", Value::from("a"))),
                placed("a", "A", ("at_end", Value::Bool(true))),
                placed("n", "N", ("before", Value::from("a"))),
            ]
        ),
        None
    );
    assert!(app.wait_until(|app| app.text("rows") == "4 rows"));
    assert_eq!(app.text("first"), "B");
    let order: Vec<String> = app
        .host
        .shared("feed", Value::Nil)
        .get()
        .list()
        .iter()
        .map(|r| match r.field("id") {
            Some(Value::Str(id)) => id.to_string(),
            _ => "-".into(),
        })
        .collect();
    // (`c` and `n` are placed before `a`, which moves to the end: they go
    // with it.)
    assert_eq!(order, ["b", "c", "n", "a"]);
    assert_eq!(
        place(&app, vec![placed("b", "B", ("before", Value::from("b")))]).as_deref(),
        Some("row 1 is placed before itself (`b`)")
    );
}

/// Text added to a shared string shows; added to a list, it is refused.
#[test]
fn a_module_appends_to_a_shared_string() {
    let mut app = reaching();
    called(&app, "reach.publish", Value::Num(1.));
    assert!(app.wait_until(|app| app.text("rows") == "1 rows"));
    let refused = called(&app, "reach.append", Value::from("one "));
    called(&app, "reach.append", Value::from("two"));
    app.settle();
    assert_eq!(app.text("log"), "one two");
    assert!(matches!(refused, Value::Str(_)), "{refused:?}");
}

/// A module hears what scripts write to a shared state, from the first
/// write after it begins to (not at once).
#[test]
fn a_module_observes_what_scripts_set() {
    let mut app = reaching();
    let heard = |app: &TestApp| format!("{:?}", called(app, "reach.heard", Value::Nil));
    let none = format!("{:?}", Value::List(Slice::from(Vec::<Value>::new())));
    assert_eq!(heard(&app), none);
    app.click("ask");
    app.click("again");
    app.settle();
    let want = format!(
        "{:?}",
        Value::List(Slice::from(vec![
            Value::from("first"),
            Value::from("second")
        ]))
    );
    assert_eq!(heard(&app), want);
}

/// A shared state's handle kept past its host reads nil and sets nothing.
#[test]
fn a_shared_handle_past_its_host_holds_nothing() {
    let mut app = reaching();
    app.click("ask");
    app.settle();
    called(&app, "reach.keep_shared", Value::Nil);
    drop(app);
    let kept = reach::KEPT_SHARED.lock().unwrap().take().unwrap().0;
    assert!(kept.get().get().is_nil());
    // Every write fails, saying its host is gone: none is told as done, nor
    // as a count of nothing changed.
    let gone = |write: Result<(), zgx_module::Failed>| match write {
        Err(why) => why.0,
        Ok(()) => "done".into(),
    };
    let row = || zgx_module::Value::record([("id", zgx_module::Value::from("r"))]);
    assert_eq!(
        gone(kept.set(zgx_module::Value::from("late")).map(drop)),
        "its host is gone"
    );
    assert_eq!(gone(kept.append("late")), "its host is gone");
    assert_eq!(gone(kept.upsert("id", [row()])), "its host is gone");
    assert_eq!(gone(kept.remove("id", ["r"]).map(drop)), "its host is gone");
}

/// A module's async action settles later, from a thread of its own: the
/// script that called (`ui.call`, or the module's table) gets the value.
#[test]
fn an_async_action_settles_later() {
    let mut app = reaching();
    app.click("later");
    assert!(
        app.wait_until(|app| app.text("said") == "got 42"),
        "{}",
        app.text("said")
    );
    app.click("member");
    assert!(
        app.wait_until(|app| app.text("said") == "member 8"),
        "{}",
        app.text("said")
    );
}

/// A promise rejected says why, after the action's and the module's names;
/// one dropped unsettled says the module dropped it.
#[test]
fn an_async_actions_rejection_names_it() {
    let mut app = reaching();
    app.click("refuse");
    assert!(app.wait_until(|app| !app.text("said").is_empty()));
    assert_eq!(
        app.text("said"),
        "reach.refuse (module `reach`): has no such row"
    );
    app.click("drop");
    assert!(app.wait_until(|app| app.text("said").contains("dropped")));
    assert_eq!(
        app.text("said"),
        "reach.drop_it (module `reach`): the module dropped its promise unsettled"
    );
}

/// A promise settled after its host is gone does nothing.
#[test]
fn a_promise_settled_past_its_host_does_nothing() {
    let mut app = reaching();
    app.click("hold");
    app.settle();
    let held = reach::HELD.lock().unwrap().take().expect("held");
    drop(app);
    // (From a thread: its job is refused, the promise dropped unrun.)
    std::thread::spawn(move || {
        held.resolve_with(|_| Ok(zgx_module::Value::from("late")));
    })
    .join()
    .unwrap();
}

/// An async action written with the macros is a member of the module's
/// table: it resolves to its record, is rejected when it says so, and over
/// a wrong argument before it runs.
#[test]
fn a_typed_async_action_resolves_its_record() {
    const SCRIPT: &str = r#"
local out, err, wrong = ui.state("out", ""), ui.state("err", ""), ui.state("wrong", "")
local typed = ui.module("typed")
typed.find_later({ id = "ab" }):after(function(f) out:set(`{f.id} {f.n}`) end)
typed.find_later({ id = "" }):after(nil, function(e) err:set(e.message) end)
typed.find_later(5):after(nil, function(e) wrong:set(e.message) end)
return ui.column {
    ui.text { id = "out", function() return out:get() end },
    ui.text { id = "err", function() return err:get() end },
    ui.text { id = "wrong", function() return wrong:get() end },
}
"#;
    let mut app = TestApp::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", SCRIPT);
        load_typed(&host);
        host
    });
    assert_eq!(app.error(), None);
    assert!(app.wait_until(|app| {
        !app.text("out").is_empty() && !app.text("err").is_empty() && !app.text("wrong").is_empty()
    }));
    assert_eq!(app.text("out"), "AB 2");
    assert_eq!(
        app.text("err"),
        "typed.find_later (module `typed`): has no id"
    );
    assert!(
        app.text("wrong")
            .starts_with("typed.find_later (module `typed`): argument 1 (query)"),
        "{}",
        app.text("wrong")
    );
}

/// A module whose functions and actions panic, by hand (`register!`).
mod panicky {
    use zgx_module::{Host, Value};
    zgx_module::register!("panicky", |host: &Host| {
        host.function("panicky.boom", |_| panic!("boom in a function"));
        host.try_function(
            "panicky.try_boom",
            |_| -> Result<Value, zgx_module::Failed> { panic!("boom in a try") },
        );
        host.function_taking("panicky.take_boom", |_| panic!("boom taking"));
        host.action("panicky.act_boom", |_| panic!("boom in an action"));
        host.try_action(
            "panicky.try_act_boom",
            |_| -> Result<(), zgx_module::Failed> { panic!("boom in a try action") },
        );
        host.async_action("panicky.later_boom", |_, _promise| {
            panic!("boom in an async action")
        });
        host.function("panicky.fine", |_| Value::from("still here"));
    });
}

/// The same, written with the macros.
#[zgx_module::module(name = "panics")]
mod panics {
    use zgx_module::export;
    /// Panics with `why`.
    #[export]
    fn boom(why: &str) -> f64 {
        panic!("{why}")
    }
    /// Panics.
    #[export(action)]
    fn act(why: &str) {
        panic!("{why}")
    }
    /// Answers, after the others panicked.
    #[export]
    fn fine() -> f64 {
        42.0
    }
}

/// A module whose setup panics.
mod setup_panics {
    zgx_module::register!("setup_panics", |host: &zgx_module::Host| {
        host.function("setup_panics.never", |_| zgx_module::Value::NIL);
        panic!("boom as it loads");
    });
}

/// A panic in a module's function or action is raised in the script that
/// called, as an error it returned would be: the app goes on, and so does the
/// module. An async action's promise rejects. A panic as a module loads
/// refuses it.
#[test]
fn a_panic_in_a_module_is_an_error_not_an_end() {
    let script = r#"
        local said = {}
        local function try(name, f)
            local ok, why = pcall(f)
            -- (An error's first line: its traceback follows.)
            table.insert(said, name .. "=" .. (if ok then "ok" else string.match(tostring(why), "^[^\n]*")))
        end
        try("function", function() return ui.call_sync("panicky.boom", 1) end)
        try("try", function() return ui.call_sync("panicky.try_boom", 1) end)
        try("taking", function() return ui.call_sync("panicky.take_boom", { 1 }) end)
        try("action", function() ui.invoke("panicky.act_boom", 1) end)
        try("try action", function() ui.invoke("panicky.try_act_boom", 1) end)
        try("macro function", function() return ui.module("panics").boom("boom by macro") end)
        try("macro action", function() ui.invoke("panics.act", "boom by macro action") end)
        table.insert(said, "after=" .. ui.call_sync("panicky.fine", 1) .. " " .. ui.module("panics").fine())
        local rejected = ui.state("rejected", "")
        ui.call("panicky.later_boom", 1):after(nil, function(e) rejected:set(tostring(e.message or e)) end)
        return ui.column {
            ui.text { table.concat(said, "\n"), id = "said" },
            ui.text { function() return rejected:get() end, id = "rejected" },
        }
    "#;
    let mut app = TestApp::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", script);
        // SAFETY: The test's own entries.
        unsafe {
            host.enter_module(panicky::zgx_module_v1, &panicky::zgx_module_manifest)
                .unwrap();
            host.enter_module(panics::zgx_module_v1, &panics::zgx_module_manifest)
                .unwrap();
        }
        host
    });
    assert_eq!(app.error(), None);
    let said = app.text("said");
    let lines: Vec<&str> = said.lines().collect();
    for (at, (name, why)) in [
        (
            "function",
            "panicky.boom (module `panicky`): panicked: boom in a function",
        ),
        (
            "try",
            "panicky.try_boom (module `panicky`): panicked: boom in a try",
        ),
        (
            "taking",
            "panicky.take_boom (module `panicky`): panicked: boom taking",
        ),
        (
            "action",
            "panicky.act_boom (module `panicky`): panicked: boom in an action",
        ),
        (
            "try action",
            "panicky.try_act_boom (module `panicky`): panicked: boom in a try action",
        ),
        (
            "macro function",
            "panics.boom (module `panics`): panicked: boom by macro",
        ),
        (
            "macro action",
            "panics.act (module `panics`): panicked: boom by macro action",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            lines[at].starts_with(&format!("{name}=")) && lines[at].contains(why),
            "{}: {:?}",
            at + 1,
            lines[at]
        );
    }
    assert_eq!(lines[7], "after=still here 42", "the modules still answer");
    assert!(app.wait_until(|app| !app.text("rejected").is_empty()));
    let rejected = app.text("rejected");
    assert!(
        rejected.contains("panicky.later_boom")
            && rejected.contains("panicked: boom in an async action"),
        "{rejected}"
    );

    let refused = TestApp::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", r#"return ui.text { "x", id = "x" }"#);
        // SAFETY: The test's own entry.
        let why = unsafe {
            host.enter_module(
                setup_panics::zgx_module_v1,
                &setup_panics::zgx_module_manifest,
            )
        }
        .unwrap_err();
        assert!(why.contains("its setup panicked"), "{why}");
        assert!(
            host.registered_function("setup_panics.never").is_none(),
            "nothing of it registered"
        );
        host
    });
    assert_eq!(refused.error(), None);
}

thread_local! {
    /// What `unruly` saw: its jobs, timers, observers and drops.
    static UNRULY: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}
thread_local! {
    /// A handle to `unruly`'s host, kept past it.
    static KEPT_UI: std::cell::RefCell<Option<zgx_module::Ui>> = const { std::cell::RefCell::new(None) };
}
fn unruly_saw(what: &str) {
    UNRULY.with_borrow_mut(|u| u.push(what.to_owned()));
}

/// A module whose jobs, timers, observers and drops panic.
mod unruly {
    use super::unruly_saw;
    use zgx_module::{Host, Value};
    /// Panics as it is dropped (a job posted to a host gone, freed unrun).
    pub struct Loud;
    impl Drop for Loud {
        fn drop(&mut self) {
            unruly_saw("dropped");
            panic!("boom in a drop");
        }
    }
    zgx_module::register!("unruly", |host: &Host| {
        let ui = host.ui();
        super::KEPT_UI.with_borrow_mut(|kept| *kept = Some(host.ui()));
        host.function("unruly.go", move |_| {
            let ui = ui.clone();
            // Two jobs: the first panics, the second still runs.
            std::thread::spawn(move || {
                ui.post(|_| panic!("boom in a job"));
                ui.post(|_| unruly_saw("the job after"));
            })
            .join()
            .unwrap();
            Value::NIL
        });
        host.after(std::time::Duration::from_millis(1), |_| {
            panic!("boom in a timer")
        })
        .forget();
        host.after(std::time::Duration::from_millis(2), |_| {
            unruly_saw("the timer after")
        })
        .forget();
        host.every(std::time::Duration::from_millis(1), |_| {
            unruly_saw("every");
            panic!("boom every time");
        })
        .forget();
        let shared = host.shared("unruly.x", Value::from(0.0)).unwrap();
        std::mem::forget(shared.observe(|_, value| {
            unruly_saw(&format!("observed {}", value.num().unwrap_or(-1.)));
            panic!("boom in an observer");
        }));
        std::mem::forget(shared);
    });
}

/// A panic in a module's job, timer, observer or drop is caught where the
/// runtime called it (printed, with no script to raise it in): the jobs and
/// timers after it run, a repeating timer that panics stops, an observer
/// observes the next change, and a closure that panics as it is dropped is
/// dropped.
#[test]
fn a_panic_off_a_call_ends_nothing() {
    UNRULY.with_borrow_mut(Vec::clear);
    let app = TestApp::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", r#"return ui.text { "x", id = "x" }"#);
        // SAFETY: The test's own entry.
        unsafe { host.enter_module(unruly::zgx_module_v1, &unruly::zgx_module_manifest) }.unwrap();
        host
    });
    app.settle();
    app.host.registered_function("unruly.go").unwrap()(Value::Nil).unwrap();
    app.frames(6);
    let shared = app.host.shared("unruly.x", 0.);
    shared.set(Value::Num(1.));
    app.frames(1);
    shared.set(Value::Num(2.));
    app.frames(1);
    let saw = UNRULY.with_borrow(Clone::clone);
    assert!(saw.contains(&"the job after".to_owned()), "{saw:?}");
    assert!(saw.contains(&"the timer after".to_owned()), "{saw:?}");
    assert_eq!(
        saw.iter().filter(|s| *s == "every").count(),
        1,
        "stopped: {saw:?}"
    );
    assert!(
        saw.contains(&"observed 1".to_owned()) && saw.contains(&"observed 2".to_owned()),
        "{saw:?}"
    );
    assert_eq!(app.error(), None);
    // A job posted once its host is gone is freed unrun, here: its drop
    // panics.
    drop(shared);
    drop(app);
    let ui = KEPT_UI.with_borrow_mut(Option::take).unwrap();
    let loud = unruly::Loud;
    assert!(!ui.post(move |_| {
        let _loud = &loud;
    }));
    assert!(
        UNRULY.with_borrow(|u| u.contains(&"dropped".to_owned())),
        "the job, freed unrun"
    );
}

/// A module's state whose drop panics, as its thread ends: caught there.
mod state_drop {
    #[derive(Default)]
    pub struct Loud;
    impl Drop for Loud {
        fn drop(&mut self) {
            panic!("boom dropping a state");
        }
    }
    zgx_module::register!("state_drop", |host: &zgx_module::Host| {
        let state = host.state::<Loud>();
        host.function("state_drop.touch", move |_| {
            state.with(|_| ());
            zgx_module::Value::NIL
        });
    });
}

#[test]
fn a_panic_dropping_a_modules_state_is_caught_as_its_thread_ends() {
    let ended = std::thread::spawn(|| {
        let app = TestApp::new(|runtime| {
            let host = ScriptHost::inline(runtime, "app", r#"return ui.text { "x", id = "x" }"#);
            // SAFETY: The test's own entry.
            unsafe {
                host.enter_module(state_drop::zgx_module_v1, &state_drop::zgx_module_manifest)
            }
            .unwrap();
            host
        });
        app.host.registered_function("state_drop.touch").unwrap()(Value::Nil).unwrap();
        drop(app);
        // The thread ends here: its states are dropped, Loud panicking.
    })
    .join();
    assert!(ended.is_ok(), "the thread ended, its state's panic caught");
}

thread_local! {
    /// How many of `tables`' functions were dropped.
    static TABLES_DROPPED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A module whose function counts its drop.
mod tables {
    use zgx_module::{Host, Value};
    struct Counted;
    impl Drop for Counted {
        fn drop(&mut self) {
            super::TABLES_DROPPED.with(|d| d.set(d.get() + 1));
        }
    }
    zgx_module::register!("tables", |host: &Host| {
        let counted = Counted;
        host.function("tables.one", move |_| {
            let _counted = &counted;
            Value::from(1.0)
        });
    });
}

/// What a module registered goes with its host: its function's drop runs
/// as the host goes, not at the thread's next load, and a host after it
/// has none of it.
#[test]
fn a_modules_functions_go_with_their_host() {
    TABLES_DROPPED.with(|d| d.set(0));
    let make = |runtime: &zgx::reactive::Runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"return ui.text { tostring(ui.module("tables").one()), id = "one" }"#,
        );
        // SAFETY: The test's own entry.
        unsafe { host.enter_module(tables::zgx_module_v1, &tables::zgx_module_manifest) }.unwrap();
        host
    };
    let mut app = TestApp::new(make);
    assert_eq!(app.text("one"), "1");
    assert_eq!(
        TABLES_DROPPED.with(std::cell::Cell::get),
        0,
        "held while its host is"
    );
    drop(app);
    assert_eq!(
        TABLES_DROPPED.with(std::cell::Cell::get),
        1,
        "dropped as its host went"
    );
    // A host made right after, in the same turn, that loads none: none of
    // the last one's is its own, wherever its functions were allocated.
    let mut fresh = TestApp::new(|runtime| {
        ScriptHost::inline(
            runtime,
            "app",
            r#"local ok, why = pcall(ui.module, "tables")
            return ui.text { tostring(ok) .. " " .. tostring(why), id = "said" }"#,
        )
    });
    let said = fresh.text("said");
    assert!(
        said.starts_with("false") && said.contains("the app loaded none"),
        "{said}"
    );
    drop(fresh);
    let mut again = TestApp::new(make);
    assert_eq!(again.text("one"), "1", "loaded again into another host");
    drop(again);
    assert_eq!(TABLES_DROPPED.with(std::cell::Cell::get), 2);
}
