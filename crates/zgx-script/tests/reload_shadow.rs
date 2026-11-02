//! What a reload keeps (#1369): while a host watches its files, a save
//! runs again, in the app's VM kept, only the modules it changed and what
//! requires them (a partial reload), or loads everything again when keeping
//! couldn't be right, and says which and why (`ScriptHost::reload_report`).
//! Keeping is asked for (`reload_partially`, `ZGX_RELOAD=partial`): by
//! default each reload loads everything and says what it would have kept.
mod common;

use common::Harness;
use std::{cell::Cell, path::PathBuf, time::Duration};
use zgx_script::{ReloadReport, ScriptHost};

/// An app in a directory of its own, its files written stamped each later
/// than the last, as saves are.
struct App {
    dir: PathBuf,
    later: Cell<u64>,
}
impl App {
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let dir =
            std::env::temp_dir().join(format!("zgx-reload-shadow-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let app = Self {
            dir,
            later: Cell::new(5),
        };
        for (file, code) in files {
            app.write(file, code);
        }
        app
    }
    fn path(&self) -> PathBuf {
        self.dir.join("app.luau")
    }
    fn write(&self, file: &str, code: &str) {
        let path = self.dir.join(file);
        std::fs::write(&path, code).unwrap();
        let later = std::time::SystemTime::now() + Duration::from_secs(self.later.get());
        self.later.set(self.later.get() + 1);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
    }
}
impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const APP: &str = r#"
local Button = require("./button")
local screen = require("./screen")
local net = require("./net")
local theme = require("./theme")
return ui.column { screen.view(), Button { label = theme.name }, ui.text { net.name } }
"#;

/// A host of `app`, watching and keeping what a save didn't change.
fn watching(app: &App) -> Harness {
    let h = Harness::new(|runtime| ScriptHost::new(runtime, app.path()));
    // Watching from now on: the load before it is not recorded.
    drop(h.host.watch(Duration::from_secs(3600)));
    h.host.reload_partially(true);
    h
}
fn last(h: &Harness) -> ReloadReport {
    h.host.reload_report().last().cloned().expect("a reload")
}
fn names(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn a_save_would_run_again_what_changed_and_what_requires_it() {
    let app = App::new(
        "partial",
        &[
            ("app.luau", APP),
            (
                "button.luau",
                "return ui.component(function(p) return ui.text { p.label } end)\n",
            ),
            (
                "screen.luau",
                "local Button = require(\"./button\")\nreturn { view = function() return Button { label = \"s\" } end }\n",
            ),
            ("net.luau", "return { name = \"net\" }\n"),
            ("theme.luau", "return { name = \"light\" }\n"),
        ],
    );
    let h = watching(&app);
    // The first reload has no record to go by: the load before it ran
    // before the host watched.
    app.write("theme.luau", "return { name = \"dark\" }\n");
    assert!(h.host.reload_if_changed());
    h.frame();
    let first = last(&h);
    assert_eq!(first.path, "full");
    assert!(
        first.why.as_deref().unwrap().contains("no record"),
        "{first:?}"
    );
    // From then on: a module, and what requires it, would run again.
    app.write(
        "button.luau",
        "return ui.component(function(p) return ui.text { p.label .. \"!\" } end)\n",
    );
    assert!(h.host.reload_if_changed());
    h.frame();
    let partial = last(&h);
    assert_eq!(partial.path, "partial", "{partial:?}");
    assert_eq!(names(&partial.changed), ["button.luau"]);
    assert_eq!(
        names(&partial.ran),
        ["app.luau", "button.luau", "screen.luau"]
    );
    assert_eq!(partial.kept, 2, "net and theme");
    // The app's own script: it alone.
    app.write("app.luau", &format!("{APP}\n-- edited\n"));
    assert!(h.host.reload_if_changed());
    h.frame();
    let own = last(&h);
    assert_eq!(names(&own.ran), ["app.luau"]);
    assert_eq!(own.kept, 4);
    assert!(own.taken && partial.taken);
    assert_eq!(h.host.reload_report().len(), 3);
}

#[test]
fn what_nothing_carries_over_would_load_everything_again() {
    let app = App::new(
        "full",
        &[
            ("app.luau", APP),
            (
                "button.luau",
                "return ui.component(function(p) return ui.text { p.label } end)\n",
            ),
            (
                "screen.luau",
                "return { view = function() return ui.text { \"s\" } end }\n",
            ),
            (
                "net.luau",
                "ui.task(function() end)\nreturn { name = \"net\" }\n",
            ),
            (
                "theme.luau",
                "Theme = \"global\"\nreturn { name = \"light\" }\n",
            ),
        ],
    );
    let h = watching(&app);
    // A first reload, recorded from here on.
    app.write(
        "screen.luau",
        "return { view = function() return ui.text { \"t\" } end }\n",
    );
    h.host.reload_if_changed();
    h.frame();
    // A task its top level starts: nothing carries one over yet.
    app.write(
        "net.luau",
        "ui.task(function() end)\nreturn { name = \"net!\" }\n",
    );
    h.host.reload_if_changed();
    h.frame();
    let task = last(&h);
    assert_eq!(task.path, "full");
    assert!(task.why.as_deref().unwrap().contains("ui.task"), "{task:?}");
    // A global its top level sets: every chunk shares them.
    app.write(
        "theme.luau",
        "Theme = \"global\"\nreturn { name = \"dark\" }\n",
    );
    h.host.reload_if_changed();
    h.frame();
    let global = last(&h);
    assert_eq!(global.path, "full");
    assert!(
        global.why.as_deref().unwrap().contains("`Theme`"),
        "{global:?}"
    );
    // A kept module may do either: it doesn't run again.
    app.write(
        "button.luau",
        "return ui.component(function(p) return ui.text { p.label .. \"?\" } end)\n",
    );
    h.host.reload_if_changed();
    h.frame();
    assert_eq!(last(&h).path, "partial", "{:?}", last(&h));
}

#[test]
fn a_host_that_never_watches_records_nothing() {
    let app = App::new(
        "unwatched",
        &[
            (
                "app.luau",
                "local m = require(\"./m\")\nreturn ui.text { m }\n",
            ),
            ("m.luau", "return \"one\"\n"),
        ],
    );
    let h = Harness::new(|runtime| ScriptHost::new(runtime, app.path()));
    app.write("m.luau", "return \"two\"\n");
    assert!(h.host.reload_if_changed());
    h.frame();
    assert!(h.host.reload_report().is_empty());
}

fn changes(h: &mut Harness, before: &str) -> String {
    h.text("log")
        .strip_prefix(before)
        .unwrap_or_else(|| panic!("{:?} after {before:?}", h.text("log")))
        .to_owned()
}

#[test]
fn a_partial_reload_keeps_what_the_save_didnt_change() {
    let app = App::new(
        "keeps",
        &[
            (
                "app.luau",
                "local token = require(\"./token\")\nlocal label = require(\"./label\")\n\
                 return ui.column { ui.text { id = \"token\", token.id }, ui.text { id = \"label\", label.text } }\n",
            ),
            ("token.luau", "return { id = tostring({}) }\n"),
            ("label.luau", "return { text = \"one\" }\n"),
        ],
    );
    let mut h = watching(&app);
    app.write("label.luau", "return { text = \"two\" }\n");
    assert!(h.host.reload_if_changed());
    h.frame();
    assert_eq!(last(&h).path, "full", "no record yet");
    let token = h.text("token");
    // Kept: the module the save didn't touch is the same table.
    app.write("label.luau", "return { text = \"three\" }\n");
    assert!(h.host.reload_if_changed());
    h.frame();
    let kept = last(&h);
    assert!(kept.path == "partial" && kept.taken, "{kept:?}");
    assert_eq!(h.text("label"), "three");
    assert_eq!(h.text("token"), token, "kept, not run again");
    // Changed: it runs again.
    app.write("token.luau", "return { id = \"new \" .. tostring({}) }\n");
    assert!(h.host.reload_if_changed());
    h.frame();
    assert_eq!(names(&last(&h).ran), ["app.luau", "token.luau"]);
    assert!(h.text("token").starts_with("new "), "{}", h.text("token"));
}

/// The states every module shares: kept, as nothing edits it.
const SHARED: &str = "return { tick = ui.state(\"tick\", 0), log = ui.state(\"log\", \"\") }\n";
/// A module whose top level makes an effect logging each run's cleanup as
/// `MARK` and the tick it saw.
const EFFECT: &str = "local s = require(\"./shared\")\n\
    ui.effect(function()\n  local n = s.tick:get()\n  ui.on_cleanup(function() s.log:set(s.log:peek() .. \"MARK\" .. n .. \" \") end)\nend)\n";
fn ticker(version: &str) -> String {
    format!(
        "{}return {{ version = \"{version}\" }}\n",
        EFFECT.replace("MARK", "x")
    )
}
fn keeper() -> String {
    format!("{}return {{}}\n", EFFECT.replace("MARK", "y"))
}
const TICKING: &str = "local s = require(\"./shared\")\nlocal t = require(\"./ticker\")\nlocal k = require(\"./keeper\")\n\
    return ui.column {\n\
    ui.text { id = \"log\", function() return s.log:get() end },\n\
    ui.text { id = \"version\", t.version },\n\
    ui.button { id = \"tick\", w = 10, h = 10, on_click = function() s.tick:set(s.tick:peek() + 1) end },\n}\n";

fn ticking(name: &str) -> (App, Harness) {
    let app = App::new(
        name,
        &[
            ("app.luau", TICKING),
            ("shared.luau", SHARED),
            ("ticker.luau", &ticker("a")),
            ("keeper.luau", &keeper()),
        ],
    );
    let h = watching(&app);
    // The first reload loads everything: no record before it.
    app.write("ticker.luau", &ticker("b"));
    assert!(h.host.reload_if_changed());
    h.frames(2);
    (app, h)
}

#[test]
fn a_module_running_again_stops_what_its_top_level_started_and_nothing_else() {
    let (app, mut h) = ticking("effects");
    let before = h.text("log");
    app.write("ticker.luau", &ticker("c"));
    assert!(h.host.reload_if_changed());
    h.frames(2);
    assert!(last(&h).taken, "{:?}", last(&h));
    assert_eq!(h.text("version"), "c");
    // Its effect went with its last run; the kept module's stays.
    assert_eq!(changes(&mut h, &before), "x0 ");
    // Each effect once: the one made again, and the one kept.
    let before = h.text("log");
    h.click("tick");
    h.frames(2);
    let ran = changes(&mut h, &before);
    assert_eq!(ran.matches('x').count(), 1, "{ran:?}");
    assert_eq!(ran.matches('y').count(), 1, "{ran:?}");
}

#[test]
fn a_failed_partial_reload_leaves_the_last_load_running_and_the_next_loads_everything() {
    let (app, mut h) = ticking("failed");
    app.write("ticker.luau", "return {\n");
    assert!(h.host.reload_if_changed());
    h.frames(2);
    assert!(last(&h).taken);
    assert!(
        h.host.error().with_untracked(Option::is_some),
        "the error shows"
    );
    assert_eq!(h.text("version"), "b", "the last load still shows");
    // Its effects still run, each once.
    let before = h.text("log");
    h.click("tick");
    h.frames(2);
    let ran = changes(&mut h, &before);
    assert_eq!(ran.matches('x').count(), 1, "{ran:?}");
    assert_eq!(ran.matches('y').count(), 1, "{ran:?}");
    app.write("ticker.luau", &ticker("d"));
    assert!(h.host.reload_if_changed());
    h.frames(2);
    let fixed = last(&h);
    assert_eq!(fixed.path, "full");
    assert!(
        fixed.why.as_deref().unwrap().contains("failed"),
        "{fixed:?}"
    );
    assert_eq!(h.text("version"), "d");
    // And the one after it keeps again.
    app.write("ticker.luau", &ticker("e"));
    assert!(h.host.reload_if_changed());
    h.frames(2);
    assert!(last(&h).taken, "{:?}", last(&h));
    assert_eq!(h.text("version"), "e");
}

#[test]
fn what_a_kept_module_declares_for_a_load_or_the_app_starts_loads_everything() {
    let (app, h) = ticking("declares");
    // A kept module's `on_mount` runs as a load mounts: kept, it wouldn't.
    app.write(
        "keeper.luau",
        &format!("ui.on_mount(function() end)\n{}", keeper()),
    );
    assert!(h.host.reload_if_changed());
    h.frame();
    app.write("ticker.luau", &ticker("c"));
    assert!(h.host.reload_if_changed());
    h.frame();
    let full = last(&h);
    assert_eq!(full.path, "full");
    assert!(
        full.why.as_deref().unwrap().contains("ui.on_mount"),
        "{full:?}"
    );
    // The app's script runs again in the VM each time: a task it starts
    // would run twice.
    app.write("keeper.luau", &keeper());
    app.write("app.luau", &format!("ui.task(function() end)\n{TICKING}"));
    assert!(h.host.reload_if_changed());
    h.frame();
    app.write("ticker.luau", &ticker("d"));
    assert!(h.host.reload_if_changed());
    h.frame();
    let task = last(&h);
    assert_eq!(task.path, "full");
    assert!(task.why.as_deref().unwrap().contains("ui.task"), "{task:?}");
}

#[test]
fn a_template_swapped_whole_loads_everything() {
    let (app, mut h) = ticking("swap");
    app.write("app.luau", &format!("-- swapped\n{TICKING}"));
    app.write("ticker.luau", &ticker("s"));
    app.write("keeper.luau", &format!("-- swapped\n{}", keeper()));
    app.write("shared.luau", &format!("-- swapped\n{SHARED}"));
    assert!(h.host.reload_if_changed());
    h.frame();
    let swap = last(&h);
    assert_eq!(swap.path, "full");
    assert!(
        swap.why.as_deref().unwrap().contains("every module"),
        "{swap:?}"
    );
    assert_eq!(h.text("version"), "s");
}

#[test]
fn hundreds_of_partial_reloads_return_to_the_memory_a_fresh_load_holds() {
    let (app, mut h) = ticking("memory");
    // What a fresh VM holds: the first reload loaded everything.
    let fresh = h.host.script_memory();
    // Each save the same size (a version of four digits), and its effect's
    // cleanup logging nothing: what's left is what the reloads keep.
    let save = |n: usize| {
        let quiet =
            ticker(&format!("{n:04}")).replace("s.log:peek() .. \"x\" .. n .. \" \"", "\"\"");
        assert!(!quiet.contains("peek"));
        app.write("ticker.luau", &quiet);
        assert!(h.host.reload_if_changed());
        h.frames(2);
        assert!(last(&h).taken, "{:?}", last(&h));
    };
    let mut after = Vec::new();
    for n in 0..600 {
        save(n);
        if n % 200 == 199 {
            after.push(h.host.script_memory());
        }
    }
    assert_eq!(h.text("version"), "0599");
    // Luau's heap is pages of 16 KB, and how full they stand after a
    // collection moves with its timing: beside other tests, a page or two
    // either way from a fresh load (alone, the same to the byte). A
    // module's run kept each save would leave hundreds of KB by now.
    assert!(
        after.iter().all(|&a| a <= fresh + 64 * 1024),
        "{fresh} bytes freshly loaded, {after:?} after 200, 400 and 600 saves"
    );
}

#[test]
fn a_host_keeps_nothing_unless_asked() {
    let (app, h) = ticking("asked");
    h.host.reload_partially(false);
    app.write("ticker.luau", &ticker("off"));
    assert!(h.host.reload_if_changed());
    h.frame();
    let shadow = last(&h);
    assert!(shadow.path == "partial" && !shadow.taken, "{shadow:?}");
}

#[test]
fn reading_a_global_zgx_makes_on_first_read_sets_nothing() {
    let (app, h) = ticking("lazy");
    // `anime` is made into the globals as a script first reads it: the
    // module reading it first doesn't set it.
    app.write("keeper.luau", &format!("local _ = anime\n{}", keeper()));
    assert!(h.host.reload_if_changed());
    h.frame();
    app.write(
        "keeper.luau",
        &format!("local _ = anime\n-- again\n{}", keeper()),
    );
    assert!(h.host.reload_if_changed());
    h.frame();
    let read = last(&h);
    assert!(read.path == "partial" && read.taken, "{read:?}");
}

#[test]
fn a_reload_the_host_asks_for_is_listed_and_not_done_twice() {
    let (app, h) = ticking("asked-for");
    let before = h.host.reload_report().len();
    app.write("ticker.luau", &ticker("asked"));
    h.host.reload().unwrap();
    h.frame();
    let asked = last(&h);
    assert_eq!(asked.path, "full");
    assert!(asked.why.as_deref().unwrap().contains("asked"), "{asked:?}");
    assert!(asked.changed.is_empty());
    assert_eq!(h.host.reload_report().len(), before + 1);
    // The watcher then finds the bytes that reload read: nothing to do.
    assert!(!h.host.reload_if_changed());
    assert_eq!(h.host.reload_report().len(), before + 1);
}
