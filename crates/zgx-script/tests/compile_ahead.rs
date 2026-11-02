//! `compile_ahead`: an app's modules compiled on a worker before its host
//! loads, used only for the very bytes `require` reads, and let go as the
//! load ends. A process of its own: what was compiled ahead is the
//! process's, one session at a time.
use std::{path::Path, time::Duration};
use zgx_script::{
    ScriptHost, compile_ahead, compile_ahead_counts, compile_ahead_pause, testing::TestApp,
};

fn write(dir: &Path, modules: usize, version: &str) {
    std::fs::create_dir_all(dir.join("mods")).unwrap();
    let mut app = String::from("local rows = {}\n");
    for i in 0..modules {
        std::fs::write(
            dir.join(format!("mods/m{i}.luau")),
            format!(
                "local M = {{}}\nfunction M.view()\n\treturn ui.text {{ \"m{i} {version}\", id = \"m{i}\" }}\nend\nreturn M\n"
            ),
        )
        .unwrap();
        app += &format!("table.insert(rows, require(\"./mods/m{i}\").view())\n");
    }
    app += "return ui.column { table.unpack(rows) }\n";
    std::fs::write(dir.join("app.luau"), app).unwrap();
}

/// One test at a time: the session is the process's.
static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn finished(files: usize) {
    for _ in 0..500 {
        let (compiled, done, ..) = compile_ahead_counts();
        if done {
            assert_eq!(compiled, files, "every file compiled ahead");
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the ahead compile never finished");
}

#[test]
fn requires_find_their_modules_compiled_ahead_only_for_the_same_bytes() {
    let _one = ONE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join(format!("zgx-compile-ahead-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(&dir, 10, "one");
    let app = dir.join("app.luau");
    let (_, _, open, hits, _) = compile_ahead_counts();
    assert!(!open, "no session open before this one");

    compile_ahead(&app, &[]);
    finished(11);
    let mut test = TestApp::new(|runtime| ScriptHost::new(runtime, &app));
    assert_eq!(test.host.error().get(), None);
    assert_eq!(test.text("m3"), "m3 one");
    let (_, _, open, _, held) = compile_ahead_counts();
    let hits = {
        let now = compile_ahead_counts().3;
        assert_eq!(
            now - hits,
            11,
            "the app and its 10 modules, each compiled ahead"
        );
        now
    };
    assert!(!open, "the session ends with the load");
    assert_eq!(held, 0, "and lets go of what it compiled");

    // Compiled ahead, then one module edited before the load: it loads
    // from its text as it is now, the rest compiled ahead.
    compile_ahead(&app, &[]);
    finished(11);
    std::fs::write(
        dir.join("mods/m3.luau"),
        "local M = {}\nfunction M.view()\n\treturn ui.text { \"m3 two\", id = \"m3\" }\nend\nreturn M\n",
    )
    .unwrap();
    let mut test = TestApp::new(|runtime| ScriptHost::new(runtime, &app));
    assert_eq!(test.host.error().get(), None);
    assert_eq!(
        test.text("m3"),
        "m3 two",
        "the bytes read, not those compiled ahead"
    );
    assert_eq!(test.text("m4"), "m4 one");
    let (_, _, open, hits_now, held) = compile_ahead_counts();
    assert_eq!(hits_now - hits, 10, "all but the edited module");
    assert!(!open);
    assert_eq!(held, 0);

    // The worker held before its first file: the load doesn't wait for
    // it, every module compiling at `require` as without it.
    compile_ahead_pause(true);
    compile_ahead(&app, &[]);
    let started = std::time::Instant::now();
    let mut test = TestApp::new(|runtime| ScriptHost::new(runtime, &app));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the load never waits for the worker"
    );
    compile_ahead_pause(false);
    assert_eq!(test.host.error().get(), None);
    assert_eq!(test.text("m3"), "m3 two");
    let (_, _, open, hits_paused, held) = compile_ahead_counts();
    assert_eq!(
        hits_paused, hits_now,
        "nothing was ready; nothing found ahead"
    );
    assert!(!open);
    assert_eq!(held, 0);

    // After the session, a load compiles at `require` as before.
    let mut test = TestApp::new(|runtime| ScriptHost::new(runtime, &app));
    assert_eq!(test.text("m3"), "m3 two");
    assert_eq!(compile_ahead_counts().3, hits_now, "nothing found ahead");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Only what the script reaches by `require` is compiled ahead: no tests,
/// type files or unused modules, and no file over the size cap. `.luaurc`
/// aliases and lent packages resolve as `require` resolves them, and a
/// second window's script joins the session.
#[test]
fn only_what_the_script_requires_is_compiled_ahead() {
    let _one = ONE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join(format!("zgx-compile-ahead-graph-{}", std::process::id()));
    let lent = dir.with_extension("lent");
    for d in [&dir, &lent] {
        let _ = std::fs::remove_dir_all(d);
    }
    let put = |path: std::path::PathBuf, code: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, code).unwrap();
    };
    put(dir.join(".luaurc"), r#"{ "aliases": { "lib": "./lib" } }"#);
    put(
        dir.join("app.luau"),
        "local a = require(\"./mods/a\")\n\
         local util = require \"@lib/util\"\n\
         local button = require(\"@ds/button\")\n\
         -- require(\"./mods/commented\")\n\
         local big = require(\"./mods/big\")\n\
         return ui.text { a .. util .. button .. big, id = \"t\" }\n",
    );
    put(dir.join("mods/a.luau"), "return require(\"./b\")\n");
    put(dir.join("mods/b.luau"), "return \"b\"\n");
    put(dir.join("lib/util.luau"), "return \"u\"\n");
    put(lent.join("button.luau"), "return \"k\"\n");
    put(
        dir.join("mods/big.luau"),
        &format!("return {:?}\n", "x".repeat(2 << 20)),
    );
    // What nothing requires.
    for unused in [
        "mods/commented.luau",
        "mods/unused.luau",
        "tests/a.luau",
        "types/a.luau",
    ] {
        put(dir.join(unused), "return \"never\"\n");
    }
    put(
        dir.join("other.luau"),
        "return ui.text { require(\"./mods/b\"), id = \"o\" }\n",
    );
    let hits = compile_ahead_counts().3;
    compile_ahead(&dir.join("app.luau"), &[("ds", &lent)]);
    // Another window's script joins the session (b again: found once).
    compile_ahead(&dir.join("other.luau"), &[]);
    // app, a, b, util, button; other, b: big is over the cap.
    finished(7);
    let mut test =
        TestApp::new(|runtime| ScriptHost::new(runtime, dir.join("app.luau")).package("ds", &lent));
    assert_eq!(test.host.error().get(), None);
    assert!(test.text("t").starts_with("buk"), "every module loaded");
    let (_, _, open, now, held) = compile_ahead_counts();
    assert_eq!(
        now - hits,
        5,
        "app, a, b, util and button compiled ahead; big at require"
    );
    assert!(!open);
    assert_eq!(held, 0);
    for d in [&dir, &lent] {
        let _ = std::fs::remove_dir_all(d);
    }
}
