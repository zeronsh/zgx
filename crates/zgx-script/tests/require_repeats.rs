//! A module an app's modules ask for again while it loads is answered from
//! what loaded (`repeats.rs`): the very same value, for the app alone, to the
//! end of the load, and never what failed, or is still loading.
mod common;

use std::{cell::Cell, path::Path, rc::Rc, time::Duration};
use zgx_script::{Capability, Plugin, ScriptHost, Value, testing::TestApp};

fn write(dir: &Path, files: &[(&str, &str)]) {
    for (name, code) in files {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, code).unwrap();
    }
}

/// `host` with `act`, which its scripts call with a number.
fn acts(host: ScriptHost, act: impl Fn(i64) + 'static) -> ScriptHost {
    host.function("act", move |n| {
        match n {
            Value::Num(n) => act(n as i64),
            other => panic!("{other:?}"),
        }
        Value::Nil
    });
    host
}
fn host(dir: &Path, act: impl Fn(i64) + 'static) -> TestApp {
    let app = dir.join("app.luau");
    TestApp::new(|runtime| acts(ScriptHost::new(runtime, &app), act))
}

#[test]
fn a_name_asked_again_answers_the_very_same_table() {
    let dir = common::temp_dir("repeats-same");
    write(
        &dir,
        &[
            (
                "ds/m.luau",
                "ui.call_sync(\"act\", 1)\nreturn { name = \"ds/m\" }",
            ),
            (
                "m.luau",
                "ui.call_sync(\"act\", 2)\nreturn { name = \"m\" }",
            ),
            ("other/m.luau", "return { name = \"other/m\" }"),
            (
                "other/c.luau",
                "return { require(\"./m\"), require(\"./m\") }",
            ),
            (
                "ds/init.luau",
                "return { mine = require(\"./m\"), self = require(\"@self/m\") }",
            ),
            (
                "ds/a.luau",
                "return { require(\"./m\"), require(\"./m\"), require(\"../m\") }",
            ),
            (
                "ds/b.luau",
                "return { require(\"./m\"), require(\"./m\"), require(\"../m\") }",
            ),
            ("ds/none.luau", "ui.call_sync(\"act\", 3)\nreturn nil"),
            (
                "app.luau",
                r#"
local a, b, ds = require("./ds/a"), require("./ds/b"), require("./ds")
local same = rawequal(a[1], a[2]) and rawequal(a[1], b[1]) and rawequal(b[1], b[2])
assert(same, "one table")
assert(rawequal(a[1], require("ds/m")) and rawequal(a[1], require("ds/m")))
assert(a[1].name == "ds/m")
local c = require("./other/c")
assert(rawequal(c[1], c[2]) and c[1].name == "other/m", "another folder's")
-- From an `init.luau`, `./m` is the one beside its folder.
assert(rawequal(ds.mine, require("m")) and ds.mine.name == "m")
assert(rawequal(ds.self, a[1]))
assert(rawequal(a[3], ds.mine) and rawequal(b[3], ds.mine))
assert(require("./ds/none") == true and require("./ds/none") == true)
assert(rawequal(require("./ds"), ds) and rawequal(require("./ds/init"), ds))
return ui.text { "shown", id = "shown" }
"#,
            ),
        ],
    );
    let ran = Rc::new(Cell::new([0; 4]));
    let counted = ran.clone();
    let mut test = host(&dir, move |n| {
        let mut now = counted.get();
        now[n as usize] += 1;
        counted.set(now);
    });
    assert_eq!(test.error(), None);
    assert_eq!(test.text("shown"), "shown");
    assert_eq!(ran.get(), [0, 1, 1, 1], "each ran once");
    test.host.reload().unwrap();
    assert_eq!(test.error(), None);
    assert_eq!(ran.get(), [0, 2, 2, 2], "a load keeps nothing of the last");
}

/// What loaded answers, and what it answers for isn't looked at again until
/// the load ends. (That it answers early shows so: the name is refused when
/// it is looked at.)
#[cfg(unix)]
#[test]
fn a_load_answers_a_name_it_asked_and_then_it_is_looked_at_again() {
    let dir = common::temp_dir("repeats-early");
    let outside = common::temp_dir("repeats-early-outside");
    write(&outside, &[("secret.luau", "return { secret = true }")]);
    write(
        &dir,
        &[
            ("m.luau", "return { mine = true }"),
            ("other.luau", "return require(\"./m\")"),
            (
                "app.luau",
                r#"
local m = require("./m")
ui.call_sync("act", 0)
-- Asked by this name, from here: answered.
assert(rawequal(require("./m"), m))
assert(rawequal(require("./other"), m), "another file, in the same folder")
-- By a name not asked yet, it is looked at, and refused.
local ok, why = pcall(require, "m")
assert(not ok and string.find(tostring(why), "modules come from", 1, true), tostring(why))
return ui.button { "ask", id = "ask", on_click = function()
	local ok, why = pcall(require, "./m")
	assert(not ok and string.find(tostring(why), "modules come from", 1, true), tostring(why))
	ui.call_sync("act", 1)
end }
"#,
            ),
        ],
    );
    let (at, asked) = (dir.clone(), Rc::new(Cell::new(false)));
    let seen = asked.clone();
    let mut test = host(&dir, move |n| match n {
        0 => {
            std::fs::remove_file(at.join("m.luau")).unwrap();
            std::os::unix::fs::symlink(outside.join("secret.luau"), at.join("m.luau")).unwrap();
        }
        _ => seen.set(true),
    });
    assert_eq!(test.error(), None);
    test.click("ask");
    assert_eq!(test.error(), None);
    assert!(asked.get(), "refused once the load was over");
}

#[test]
fn a_module_that_failed_is_asked_again() {
    let dir = common::temp_dir("repeats-failed");
    write(
        &dir,
        &[
            ("m.luau", "error(\"not yet\")"),
            (
                "app.luau",
                r#"
for _ = 1, 3 do
	local ok, why = pcall(require, "./m")
	assert(not ok and string.find(tostring(why), "not yet", 1, true), tostring(why))
end
ui.call_sync("act", 0)
local m = require("./m")
assert(m.now == true and rawequal(m, require("./m")))
return ui.text { "shown", id = "shown" }
"#,
            ),
        ],
    );
    let at = dir.join("m.luau");
    let mut test = host(&dir, move |_| {
        std::fs::write(&at, "return { now = true }").unwrap();
    });
    assert_eq!(test.error(), None);
    assert_eq!(test.text("shown"), "shown");
}

#[test]
fn a_cycle_says_so_each_time() {
    let dir = common::temp_dir("repeats-cycle");
    write(
        &dir,
        &[
            (
                "a.luau",
                r#"
local said = {}
for at = 1, 3 do
	local ok, why = pcall(require, "./b")
	assert(not ok)
	said[at] = tostring(why)
end
-- Itself, while it loads.
local _, mine = pcall(require, "./a")
return { said = said, mine = tostring(mine) }
"#,
            ),
            ("b.luau", "return require(\"./a\")"),
            (
                "app.luau",
                r#"
local a = require("./a")
local cycle = "require(\"./a\"): a require cycle"
for _, why in a.said do
	assert(string.find(why, cycle, 1, true), why)
end
assert(#a.said == 3)
assert(string.find(a.mine, cycle, 1, true), a.mine)
-- Loaded, it is no cycle: and `b`, which failed, loads.
assert(rawequal(require("./b"), a) and rawequal(require("./a"), a))
return ui.text { "shown", id = "shown" }
"#,
            ),
        ],
    );
    let mut test = host(&dir, |_| {});
    assert_eq!(test.error(), None);
    assert_eq!(test.text("shown"), "shown");
}

/// A hot-reloading host loads a module that changed, however often it was
/// asked, and who asked is known (a partial reload's).
#[test]
fn a_reload_takes_a_module_that_changed() {
    let dir = common::temp_dir("repeats-reload");
    write(
        &dir,
        &[
            ("m.luau", "return { n = \"one\" }"),
            ("a.luau", "return { m = require(\"./m\") }"),
            ("b.luau", "return { m = require(\"./m\") }"),
            (
                "app.luau",
                r#"
local a, b = require("./a"), require("./b")
assert(rawequal(a.m, b.m) and rawequal(a.m, require("./m")))
return ui.text { a.m.n .. " " .. b.m.n, id = "shown" }
"#,
            ),
        ],
    );
    let mut test = host(&dir, |_| {});
    assert_eq!(test.error(), None);
    assert_eq!(test.text("shown"), "one one");
    for n in ["two", "three"] {
        write(&dir, &[("m.luau", &format!("return {{ n = \"{n}\" }}"))]);
        test.host.reload().unwrap();
        test.settle();
        assert_eq!(test.error(), None);
        assert_eq!(test.text("shown"), format!("{n} {n}"));
    }
}

/// Who asked again is recorded as who asked first is: each runs again when
/// what it requires changes, and alone.
#[test]
fn a_partial_reload_runs_again_what_asked_again() {
    let dir = common::temp_dir("repeats-partial");
    let files = |n: &str| {
        write(&dir, &[("m.luau", &format!("return {{ n = \"{n}\" }}"))]);
        // (Later than any write before it, as a save is.)
        let secs = n.len() as u64 * 5;
        std::fs::File::options()
            .write(true)
            .open(dir.join("m.luau"))
            .unwrap()
            .set_modified(std::time::SystemTime::now() + Duration::from_secs(secs))
            .unwrap();
    };
    write(
        &dir,
        &[
            ("a.luau", "return { m = require(\"./m\") }"),
            ("b.luau", "return { m = require(\"./m\") }"),
            ("c.luau", "return { n = \"c\" }"),
            (
                "app.luau",
                r#"
local a, b, c = require("./a"), require("./b"), require("./c")
return ui.text { a.m.n .. " " .. b.m.n .. " " .. c.n, id = "shown" }
"#,
            ),
        ],
    );
    files("1");
    let mut test = host(&dir, |_| {});
    drop(test.host.watch(Duration::from_secs(3600)));
    test.host.reload_partially(true);
    // The first has no record to go by.
    for n in ["22", "333", "4444"] {
        files(n);
        assert!(test.host.reload_if_changed());
        test.frame();
        assert_eq!(test.error(), None);
        assert_eq!(test.text("shown"), format!("{n} {n} c"));
    }
    let last = test.host.reload_report().last().cloned().unwrap();
    assert!(last.path == "partial" && last.taken, "{last:?}");
    let ran: Vec<_> = last.ran.iter().filter_map(|p| p.file_name()).collect();
    assert_eq!(ran, ["a.luau", "app.luau", "b.luau", "m.luau"]);
}

/// A package's folder replaced by another while the app loads: what loaded
/// answers, as it did, and what hasn't comes from the one that is there.
#[test]
fn a_package_replaced_mid_load_answers_as_it_did() {
    let dir = common::temp_dir("repeats-package");
    let packages = common::temp_dir("repeats-package-of");
    write(
        &packages,
        &[
            ("kit/m.luau", "return { kit = \"one\" }"),
            ("kit/later.luau", "return { kit = \"one\" }"),
            ("next/m.luau", "return { kit = \"two\" }"),
            ("next/later.luau", "return { kit = \"two\" }"),
        ],
    );
    write(
        &dir,
        &[(
            "app.luau",
            r#"
local m = require("@kit/m")
assert(rawequal(m, require("@kit/m")) and m.kit == "one")
ui.call_sync("act", 0)
assert(rawequal(m, require("@kit/m")), "what loaded")
assert(require("@kit/later").kit == "two")
return ui.text { "shown", id = "shown" }
"#,
        )],
    );
    let app = dir.join("app.luau");
    let at = packages.clone();
    let mut test = TestApp::new(|runtime| {
        let host = ScriptHost::new(runtime, &app).package("kit", packages.join("kit"));
        acts(host, move |_| {
            std::fs::rename(at.join("kit"), at.join("was")).unwrap();
            std::fs::rename(at.join("next"), at.join("kit")).unwrap();
        })
    });
    assert_eq!(test.error(), None);
    assert_eq!(test.text("shown"), "shown");
}

/// A plugin's requires are looked at each time, a lent package's among them.
#[cfg(unix)]
#[test]
fn a_plugins_requires_are_looked_at_each_time() {
    let dir = common::temp_dir("repeats-plugin");
    let outside = common::temp_dir("repeats-plugin-outside");
    write(&outside, &[("secret.luau", "return { secret = true }")]);
    write(
        &dir,
        &[
            ("app/app.luau", "return ui.text { \"app\", id = \"app\" }"),
            ("kit/m.luau", "return { kit = true }"),
            ("plugin/m.luau", "return { mine = true }"),
            (
                "plugin/plugin.luau",
                r#"
local m, kit = require("./m"), require("@kit/m")
assert(rawequal(require("./m"), m) and rawequal(require("@kit/m"), kit))
ui.call_sync("act", 0)
for _, name in { "./m", "@kit/m" } do
	local ok, why = pcall(require, name)
	assert(not ok, name .. " was answered")
end
ui.call_sync("act", 1)
"#,
            ),
        ],
    );
    let asked = Rc::new(Cell::new(false));
    let (seen, at) = (asked.clone(), dir.clone());
    let app = dir.join("app/app.luau");
    let test = TestApp::new(|runtime| {
        let host = ScriptHost::new(runtime, &app).package("kit", dir.join("kit"));
        host.expose_action("act", None);
        host.set_plugins(vec![Plugin {
            name: "p".into(),
            entry: dir.join("plugin/plugin.luau"),
            grants: vec![Capability::Package("kit".into())],
        }]);
        acts(host, move |n| match n {
            0 => {
                for file in ["plugin/m.luau", "kit/m.luau"] {
                    std::fs::remove_file(at.join(file)).unwrap();
                    let secret = outside.join("secret.luau");
                    std::os::unix::fs::symlink(secret, at.join(file)).unwrap();
                }
            }
            _ => seen.set(true),
        })
    });
    assert_eq!(test.error(), None);
    assert!(asked.get(), "the plugin ran to its end");
}
