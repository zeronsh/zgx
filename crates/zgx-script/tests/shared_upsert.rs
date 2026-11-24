//! A shared state's keyed rows written without its whole list
//! (`Shared::upsert`, `Shared::remove_rows`, and a native module's): only
//! the rows given are new, the rest shared; one pass over the list.

use zgx_script::{ScriptHost, Shared, Slice, Value, testing::TestApp};

fn row(id: &str, title: &str) -> Value {
    Value::record([("id", Value::from(id)), ("title", Value::from(title))])
}
fn rows(list: &Value) -> Vec<String> {
    list.list()
        .iter()
        .map(|r| {
            let field = |k: &str| match r.field(k) {
                Some(Value::Str(s)) => s.to_string(),
                _ => "-".into(),
            };
            format!("{}:{}", field("id"), field("title"))
        })
        .collect()
}
fn feed(app: &TestApp) -> Shared {
    app.host.shared("feed", Value::Nil)
}

const SCRIPT: &str = r#"
local feed = ui.shared("feed")
return ui.text { id = "shown", function()
    local parts = {}
    for _, r in feed:get() or {} do table.insert(parts, `{r.id}:{r.title}`) end
    return table.concat(parts, " ")
end }
"#;

fn app() -> TestApp {
    TestApp::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", SCRIPT);
        host.shared(
            "feed",
            Value::List(Slice::from(vec![
                row("a", "one"),
                row("b", "two"),
                row("c", "three"),
            ])),
        );
        host
    })
}

/// Rows given by key replace theirs where they are, new ones go at the
/// end, and what scripts show follows; removed rows go.
#[test]
fn keyed_rows_replace_theirs_and_new_ones_follow() {
    let mut app = app();
    assert_eq!(app.text("shown"), "a:one b:two c:three");
    assert!(
        feed(&app)
            .upsert("id", vec![row("b", "TWO"), row("d", "four")])
            .unwrap()
    );
    app.settle();
    assert_eq!(app.text("shown"), "a:one b:TWO c:three d:four");
    assert!(feed(&app).remove_rows("id", &["a", "c"]));
    app.settle();
    assert_eq!(app.text("shown"), "b:TWO d:four");
    assert!(!feed(&app).remove_rows("id", &["zz"]), "nothing to remove");
    assert!(
        !feed(&app).upsert("id", vec![row("b", "TWO")]).unwrap(),
        "equal: no change"
    );
}

/// The rows not given are the very rows the list held (shared, not
/// copied); a row given equal to its own keeps its own.
#[test]
fn the_rows_not_given_are_shared() {
    let app = app();
    let before = feed(&app).get();
    feed(&app).upsert("id", vec![row("b", "new")]).unwrap();
    let after = feed(&app).get();
    let (Value::List(before), Value::List(after)) = (&before, &after) else {
        panic!("lists")
    };
    let same = |i: usize| match (&before[i], &after[i]) {
        (Value::Map(a), Value::Map(b)) => Slice::ptr_eq(a, b),
        _ => false,
    };
    assert!(same(0) && same(2), "a and c are the same rows");
    assert!(!same(1), "b is new");
}

/// Held by nothing else, the list is written in place: the same list,
/// its one row new.
#[test]
fn a_list_held_nowhere_else_is_written_in_place() {
    let app = TestApp::new(|runtime| ScriptHost::inline(runtime, "app", "return ui.column {}"));
    let shared = app.host.shared(
        "rows",
        Value::List(Slice::from(vec![row("a", "one"), row("b", "two")])),
    );
    let list_of = |shared: &Shared| {
        shared.signal().with_untracked(|v| match v {
            Value::List(l) => l.as_ptr() as usize,
            _ => 0,
        })
    };
    let before = list_of(&shared);
    assert!(shared.upsert("id", vec![row("a", "ONE")]).unwrap());
    assert_eq!(list_of(&shared), before, "the same list");
    assert_eq!(rows(&shared.get()), ["a:ONE", "b:two"]);
}

/// Of two rows given with one key the last is written; of two rows in the
/// list with one key the first is replaced, and removing removes both.
#[test]
fn duplicate_keys_are_said() {
    let app = TestApp::new(|runtime| ScriptHost::inline(runtime, "app", "return ui.column {}"));
    let shared = app.host.shared(
        "rows",
        Value::List(Slice::from(vec![
            row("a", "first a"),
            row("b", "b"),
            row("a", "second a"),
        ])),
    );
    shared
        .upsert(
            "id",
            vec![
                row("a", "given 1"),
                row("c", "c 1"),
                row("a", "given 2"),
                row("c", "c 2"),
            ],
        )
        .unwrap();
    assert_eq!(
        rows(&shared.get()),
        ["a:given 2", "b:b", "a:second a", "c:c 2"]
    );
    assert!(shared.remove_rows("id", &["a"]));
    assert_eq!(rows(&shared.get()), ["b:b", "c:c 2"]);
}

/// A row with no string at the key is refused, naming it, and nothing is
/// written.
#[test]
fn a_row_without_its_key_is_refused() {
    let app = app();
    let refused = feed(&app)
        .upsert(
            "id",
            vec![
                row("b", "kept out"),
                Value::record([("title", Value::from("no id"))]),
            ],
        )
        .unwrap_err();
    assert_eq!(refused, "row 2 has no string `id`");
    assert_eq!(rows(&feed(&app).get()), ["a:one", "b:two", "c:three"]);
}

/// Rows given a state that holds no list yet make it one; one that holds
/// something else refuses them.
#[test]
fn rows_make_a_list_of_nothing_and_refuse_anything_else() {
    let app = TestApp::new(|runtime| ScriptHost::inline(runtime, "app", "return ui.column {}"));
    let empty = app.host.shared("empty", Value::Nil);
    assert!(empty.upsert("id", vec![row("a", "one")]).unwrap());
    assert_eq!(rows(&empty.get()), ["a:one"]);
    let text = app.host.shared("text", Value::from("words"));
    assert_eq!(
        text.upsert("id", vec![row("a", "one")]).unwrap_err(),
        "it holds no list"
    );
    assert!(matches!(text.get(), Value::Str(s) if &*s == "words"));
}

/// A long list and a burst of rows: one pass, so 500 rows into 10,000 is
/// about one walk of the list (a scan per row would be 500 walks).
#[test]
fn a_burst_into_a_long_list_is_one_pass() {
    let app = TestApp::new(|runtime| ScriptHost::inline(runtime, "app", "return ui.column {}"));
    let long: Vec<Value> = (0..10_000).map(|i| row(&format!("r{i}"), "x")).collect();
    let shared = app.host.shared("rows", Value::List(Slice::from(long)));
    let burst: Vec<Value> = (0..500)
        .map(|i| row(&format!("r{}", i * 20), "burst"))
        .collect();
    let started = std::time::Instant::now();
    assert!(shared.upsert("id", burst).unwrap());
    let took = started.elapsed();
    let written = rows(&shared.get());
    assert_eq!(written.len(), 10_000);
    assert_eq!(written[20], "r20:burst");
    assert_eq!(written[21], "r21:x");
    // (A scan per row, 5,000,000 key compares, takes tens of milliseconds
    // in a debug build; one pass, about 10,000 lookups, well under one.)
    assert!(took < std::time::Duration::from_millis(250), "{took:?}");
}
