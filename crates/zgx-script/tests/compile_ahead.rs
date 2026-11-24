//! `compile_ahead`: an app's modules compiled on a worker before its host
//! loads, used only for the very bytes `require` reads, and let go as the
//! load ends. A process of its own: what was compiled ahead is the
//! process's, one session at a time.
mod common;

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
    let dir = common::temp_dir("compile-ahead");
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
/// type files or unused modules, and no file over the size cap (16 MB; a
/// data module of megabytes is under it). `.luaurc`
/// aliases and lent packages resolve as `require` resolves them, and a
/// second window's script joins the session.
#[test]
fn only_what_the_script_requires_is_compiled_ahead() {
    let _one = ONE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = common::temp_dir("compile-ahead-graph");
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
         local huge = require(\"./mods/huge\")\n\
         return ui.text { a .. util .. button .. #big .. \" \" .. #huge, id = \"t\" }\n",
    );
    put(dir.join("mods/a.luau"), "return require(\"./b\")\n");
    put(dir.join("mods/b.luau"), "return \"b\"\n");
    put(dir.join("lib/util.luau"), "return \"u\"\n");
    put(lent.join("button.luau"), "return \"k\"\n");
    put(
        dir.join("mods/big.luau"),
        &format!("return {:?}\n", "x".repeat(2 << 20)),
    );
    put(
        dir.join("mods/huge.luau"),
        &format!("return {:?}\n", "x".repeat(17 << 20)),
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
    // Another window's script joins the session (b again: compiled once).
    compile_ahead(&dir.join("other.luau"), &[]);
    // app, a, b, util, button, big; other: huge is over the cap.
    finished(7);
    let mut test =
        TestApp::new(|runtime| ScriptHost::new(runtime, dir.join("app.luau")).package("ds", &lent));
    assert_eq!(test.host.error().get(), None);
    assert_eq!(
        test.text("t"),
        format!("buk{} {}", 2 << 20, 17 << 20),
        "every module loaded"
    );
    let (_, _, open, now, held) = compile_ahead_counts();
    assert_eq!(
        now - hits,
        6,
        "app, a, b, util, button and big compiled ahead; huge at require"
    );
    assert!(!open);
    assert_eq!(held, 0);
    for d in [&dir, &lent] {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// A string that names a module under a known alias leads to it, however
/// it is required: `pcall(require, …)`, a cast, a function of the script's
/// own, a list. Not one in a comment, one naming no file or an alias there
/// isn't, and none that would lead out of the script's folder: by `..`, by a
/// link, or by an alias `require` refuses.
#[test]
fn a_string_that_names_a_module_is_compiled_ahead() {
    let _one = ONE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let base = common::temp_dir("compile-ahead-strings");
    let _ = std::fs::remove_dir_all(&base);
    let dir = base.join("app");
    let put = |path: std::path::PathBuf, code: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, code).unwrap();
    };
    put(
        dir.join(".luaurc"),
        r#"{ "aliases": { "lib": "./lib", "away": "../away" } }"#,
    );
    put(
        dir.join("app.luau"),
        "local ok, caught = pcall(require, \"@app/mods/caught\")\n\
         local cast = (require :: any)('@app/mods/cast')\n\
         local function load(name) return (require :: any)(name) end\n\
         local all = {}\n\
         for _, name in { \"@app/mods/listed\", \"@lib/folder\" } do table.insert(all, load(name)) end\n\
         -- load(\"@app/mods/commented\")\n\
         local words = { \"@app/none\", \"@nope/mods/cast\", \"@app/../outside\", \"@away/there\", \"@app/link\", \"me@app/mods/never\", \"@app/mods/never here\" }\n\
         return ui.text { caught .. cast .. table.concat(all) .. #words, id = \"t\" }\n",
    );
    for (name, says) in [
        ("mods/caught", "a"),
        ("mods/cast", "b"),
        ("mods/listed", "c"),
        ("lib/folder/init", "d"),
        ("mods/commented", "never"),
        ("mods/never", "never"),
    ] {
        put(
            dir.join(format!("{name}.luau")),
            &format!("return \"{says}\"\n"),
        );
    }
    put(base.join("outside.luau"), "return \"out\"\n");
    put(base.join("away/there.luau"), "return \"out\"\n");
    #[cfg(unix)]
    std::os::unix::fs::symlink(base.join("outside.luau"), dir.join("link.luau")).unwrap();
    let app = dir.join("app.luau");
    let hits = compile_ahead_counts().3;
    compile_ahead(&app, &[]);
    // app, caught, cast, listed and folder/init.
    finished(5);
    let mut test = TestApp::new(|runtime| ScriptHost::new(runtime, &app));
    assert_eq!(test.host.error().get(), None);
    assert_eq!(test.text("t"), "abcd7");
    let (_, _, open, now, held) = compile_ahead_counts();
    assert_eq!(now - hits, 5, "each found compiled ahead");
    assert!(!open);
    assert_eq!(held, 0);
    let _ = std::fs::remove_dir_all(&base);
}

/// Live bytes of every allocation, for what a load leaves.
struct Counting;
static LIVE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
// SAFETY: The system's, counted.
unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, std::sync::atomic::Ordering::Relaxed);
        // SAFETY: As asked.
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, at: *mut u8, layout: std::alloc::Layout) {
        LIVE.fetch_sub(layout.size() as isize, std::sync::atomic::Ordering::Relaxed);
        // SAFETY: As asked.
        unsafe { std::alloc::System.dealloc(at, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The ahead workers of this process.
#[cfg(target_os = "linux")]
fn workers() -> usize {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .flatten()
        .filter(|task| {
            std::fs::read_to_string(task.path().join("comm"))
                .is_ok_and(|name| name.trim() == "zgx-luau-ahead")
        })
        .count()
}

/// Several workers compile a script's files, each file once, and after the
/// load neither they nor what they compiled is left: modules named and never
/// required among it.
#[test]
fn workers_and_what_no_load_took_go_as_the_load_ends() {
    let _one = ONE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = common::temp_dir("compile-ahead-many");
    let _ = std::fs::remove_dir_all(&dir);
    const MODULES: usize = 120;
    write(&dir, MODULES, "one");
    // As many again, named and never required.
    for i in 0..MODULES {
        std::fs::write(
            dir.join(format!("mods/unused{i}.luau")),
            format!(
                "return {{ {} }}\n",
                "\"a string of some length\", ".repeat(400)
            ),
        )
        .unwrap();
    }
    let names: String = (0..MODULES)
        .map(|i| format!("\"@app/mods/unused{i}\", "))
        .collect();
    let body = std::fs::read_to_string(dir.join("app.luau")).unwrap();
    std::fs::write(
        dir.join("app.luau"),
        format!("local names = {{ {names} }}\n{body}"),
    )
    .unwrap();
    let app = dir.join("app.luau");
    let settled = |ahead: bool| {
        if ahead {
            compile_ahead(&app, &[]);
            finished(1 + 2 * MODULES);
        }
        let before = LIVE.load(std::sync::atomic::Ordering::Relaxed);
        let mut test = TestApp::new(|runtime| ScriptHost::new(runtime, &app));
        assert_eq!(test.host.error().get(), None);
        assert_eq!(test.text("m3"), "m3 one");
        assert_eq!(compile_ahead_counts().4, 0, "nothing held");
        #[cfg(target_os = "linux")]
        for _ in 0..500 {
            if workers() == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        #[cfg(target_os = "linux")]
        assert_eq!(workers(), 0, "no worker left");
        #[cfg(not(target_os = "linux"))]
        std::thread::sleep(Duration::from_millis(500));
        let after = LIVE.load(std::sync::atomic::Ordering::Relaxed);
        drop(test);
        (before, after)
    };
    // (A first of each, for what is made once.)
    settled(false);
    settled(true);
    let (_, without) = settled(false);
    let gone = LIVE.load(std::sync::atomic::Ordering::Relaxed);
    let (_, with) = settled(true);
    let left = LIVE.load(std::sync::atomic::Ordering::Relaxed);
    // What it compiled is megabytes; what a load leaves varies by some KB.
    assert!(
        (with - without).abs() < 64 << 10 && (left - gone).abs() < 64 << 10,
        "compiled ahead, a load leaves {} bytes more, and {} once its app is gone",
        with - without,
        left - gone
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A data module of megabytes (a feed in a string) is compiled ahead, and
/// what was compiled for it is let go as soon as its `require` has loaded
/// it, not held to the end of the load, through all the app does with it.
#[test]
fn a_module_is_let_go_once_its_require_has_loaded_it() {
    let _one = ONE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join(format!("zgx-compile-ahead-held-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("data.luau"),
        format!("return [==[{}]==]\n", "{\"m\":1}".repeat(400_000)),
    )
    .unwrap();
    std::fs::write(dir.join("rest.luau"), "return \"rest\"\n").unwrap();
    std::fs::write(
        dir.join("app.luau"),
        "local data = require(\"./data\")\n\
         local after_data = ui.call_sync(\"held\")\n\
         local rest = require(\"./rest\")\n\
         local after_rest = ui.call_sync(\"held\")\n\
         return ui.text { #data .. \" \" .. after_data .. \" \" .. after_rest .. \" \" .. rest, id = \"t\" }\n",
    )
    .unwrap();
    let app = dir.join("app.luau");
    let hits = compile_ahead_counts().3;
    compile_ahead(&app, &[]);
    finished(3);
    let mut test = TestApp::new(|runtime| {
        let host = ScriptHost::new(runtime, &app);
        host.function("held", |_| {
            zgx_script::Value::Num(compile_ahead_counts().4 as f64)
        });
        host
    });
    assert_eq!(test.host.error().get(), None);
    assert_eq!(
        test.text("t"),
        "2800000 1 0 rest",
        "after data's require only rest is held; after rest's, nothing"
    );
    let (_, _, open, now, held) = compile_ahead_counts();
    assert_eq!(
        now - hits,
        3,
        "the app, its 2.8 MB data and rest, all compiled ahead"
    );
    assert!(!open);
    assert_eq!(held, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the worker compiled comes back to it as the load lets it go, and it
/// rests its own thread (`zgx::image::rest_thread`): a host whose
/// allocator keeps a thread's freed pages for that thread gets them back.
#[test]
fn the_worker_rests_once_what_it_made_comes_back() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let _one = ONE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    static ON_WORKER: AtomicUsize = AtomicUsize::new(0);
    static ELSEWHERE: AtomicUsize = AtomicUsize::new(0);
    fn rest() {
        if std::thread::current().name() == Some("zgx-luau-ahead") {
            ON_WORKER.fetch_add(1, Ordering::Relaxed);
        } else {
            ELSEWHERE.fetch_add(1, Ordering::Relaxed);
        }
    }
    zgx::image::set_decoder_rest(rest);
    let dir = std::env::temp_dir().join(format!("zgx-compile-ahead-rest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(&dir, 3, "one");
    let app = dir.join("app.luau");
    let (before, elsewhere) = (
        ON_WORKER.load(Ordering::Relaxed),
        ELSEWHERE.load(Ordering::Relaxed),
    );
    compile_ahead(&app, &[]);
    finished(4);
    let test = TestApp::new(|runtime| ScriptHost::new(runtime, &app));
    assert_eq!(test.host.error().get(), None);
    drop(test);
    // Once (at least) after what it made came back.
    let started = std::time::Instant::now();
    while ON_WORKER.load(Ordering::Relaxed) < before + 1 {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the worker rested {} times",
            ON_WORKER.load(Ordering::Relaxed) - before
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        ELSEWHERE.load(Ordering::Relaxed),
        elsewhere,
        "only the worker rests: the loading thread holds none of its pages"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
