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

mod placed {
    //! Rows given a place (`Shared::upsert_at`): before a key's row, at the
    //! end, or where they are.
    use super::{row, rows};
    use zgx_script::{Place, ScriptHost, Shared, Slice, Value, testing::TestApp};

    fn listed(app: &TestApp, ids: &[&str]) -> Shared {
        app.host.shared(
            "rows",
            Value::List(Slice::from(
                ids.iter().map(|id| row(id, "x")).collect::<Vec<_>>(),
            )),
        )
    }
    fn app() -> TestApp {
        TestApp::new(|runtime| ScriptHost::inline(runtime, "app", "return ui.column {}"))
    }
    fn before(key: &str) -> Place {
        Place::Before(key.into())
    }
    fn order(shared: &Shared) -> String {
        rows(&shared.get())
            .iter()
            .map(|r| r.split(':').next().unwrap().to_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// A row of the list placed at the end moves there; the rest stay the
    /// very rows they were.
    #[test]
    fn a_row_moves_to_the_end() {
        let app = app();
        let shared = listed(&app, &["a", "b", "c"]);
        let Value::List(was) = shared.get() else {
            panic!()
        };
        assert!(
            shared
                .upsert_at("id", vec![(row("a", "x"), Place::End)])
                .unwrap()
        );
        assert_eq!(order(&shared), "b c a");
        let Value::List(now) = shared.get() else {
            panic!()
        };
        let same = |a: &Value, b: &Value| matches!((a, b), (Value::Map(a), Value::Map(b)) if Slice::ptr_eq(a, b));
        assert!(
            same(&now[0], &was[1]) && same(&now[1], &was[2]),
            "b and c are the rows they were"
        );
        assert!(same(&now[2], &was[0]), "a, given equal, is the row it was");
        assert!(
            !shared
                .upsert_at("id", vec![(row("a", "x"), Place::End)])
                .unwrap(),
            "there already"
        );
    }

    /// A row moved and a row inserted before one key go before it in the
    /// order given, and a row moved up is reconciled with its old one.
    #[test]
    fn a_move_and_an_insert_share_an_anchor() {
        let app = app();
        let shared = listed(&app, &["a", "b", "c", "d"]);
        let given = vec![
            (row("d", "moved"), before("b")),
            (row("x", "new"), before("b")),
        ];
        assert!(shared.upsert_at("id", given).unwrap());
        assert_eq!(
            rows(&shared.get()),
            ["a:x", "d:moved", "x:new", "b:x", "c:x"]
        );
    }

    /// A row placed before itself is refused, nothing written.
    #[test]
    fn a_row_before_itself_is_refused() {
        let app = app();
        let shared = listed(&app, &["a", "b"]);
        let refused = shared.upsert_at(
            "id",
            vec![(row("b", "y"), Place::End), (row("a", "y"), before("a"))],
        );
        assert_eq!(refused.unwrap_err(), "row 2 is placed before itself (`a`)");
        assert_eq!(rows(&shared.get()), ["a:x", "b:x"]);
    }

    /// Rows placed before one another are refused, saying which, and a
    /// row placed before a key no one has; nothing written either way.
    #[test]
    fn a_cycle_or_a_gone_anchor_is_refused() {
        let app = app();
        let shared = listed(&app, &["a", "b", "c"]);
        let cycle = vec![
            (row("a", "y"), before("b")),
            (row("b", "y"), before("c")),
            (row("c", "y"), before("a")),
        ];
        assert_eq!(
            shared.upsert_at("id", cycle).unwrap_err(),
            "rows are placed before one another: `a` before `b` before `c` before `a`"
        );
        let gone = vec![(row("x", "y"), Place::End), (row("y", "y"), before("zz"))];
        assert_eq!(
            shared.upsert_at("id", gone).unwrap_err(),
            "row 2 is placed before `zz`, which isn't in the list"
        );
        assert_eq!(rows(&shared.get()), ["a:x", "b:x", "c:x"]);
    }

    /// New rows each placed before the next (as a module that diffs a
    /// reordered list posts them) go in that order before the last's
    /// anchor; a row that stays is written where it is; new rows that stay
    /// go at the end.
    #[test]
    fn a_chain_of_rows_goes_in_order() {
        let app = app();
        let shared = listed(&app, &["a", "b", "c"]);
        let given = vec![
            (row("y", "n"), before("c")),
            (row("x", "n"), before("y")),
            (row("b", "B"), Place::Stay),
            (row("z", "n"), Place::Stay),
        ];
        assert!(shared.upsert_at("id", given).unwrap());
        assert_eq!(order(&shared), "a b x y c z");
        assert_eq!(rows(&shared.get())[1], "b:B");
        // Before a row that moves itself: after it, where it goes.
        let given = vec![(row("a", "x"), Place::End), (row("q", "n"), before("a"))];
        shared.upsert_at("id", given).unwrap();
        assert_eq!(order(&shared), "b x y c z q a");
    }

    /// Rows that all stay are written as `upsert` writes them: in place,
    /// the list held nowhere else.
    #[test]
    fn rows_that_stay_are_written_in_place() {
        let app = app();
        let shared = listed(&app, &["a", "b"]);
        let list_of = |shared: &Shared| {
            shared.signal().with_untracked(|v| match v {
                Value::List(l) => l.as_ptr() as usize,
                _ => 0,
            })
        };
        let before = list_of(&shared);
        assert!(
            shared
                .upsert_at("id", vec![(row("b", "B"), Place::Stay)])
                .unwrap()
        );
        assert_eq!(list_of(&shared), before, "the same list");
        assert_eq!(rows(&shared.get()), ["a:x", "b:B"]);
    }

    /// A long burst of rows, each before the next, into a long list: one
    /// walk, and no stack for the chain.
    #[test]
    fn a_burst_of_placed_rows_is_one_walk() {
        let app = app();
        let ids: Vec<String> = (0..10_000).map(|i| format!("r{i:05}")).collect();
        let shared = listed(&app, &ids.iter().map(String::as_str).collect::<Vec<_>>());
        // 500 new rows, spread through it, and a chain of 10,000 at its end.
        let mut given: Vec<(Value, Place)> = (0..500)
            .map(|i| (row(&format!("n{i}"), "n"), before(&ids[i * 20])))
            .collect();
        given.extend((0..10_000).map(|i| {
            (
                row(&format!("c{i}"), "c"),
                if i == 9_999 {
                    Place::End
                } else {
                    before(&format!("c{}", i + 1))
                },
            )
        }));
        let started = std::time::Instant::now();
        assert!(shared.upsert_at("id", given).unwrap());
        let took = started.elapsed();
        let order = rows(&shared.get());
        assert_eq!(order.len(), 20_500);
        assert_eq!(
            (&*order[0], &*order[1], &*order[21], &*order[22]),
            ("n0:n", "r00000:x", "n1:n", "r00020:x")
        );
        assert_eq!((&*order[10_500], &*order[20_499]), ("c0:c", "c9999:c"));
        assert!(took < std::time::Duration::from_millis(500), "{took:?}");
    }
}
