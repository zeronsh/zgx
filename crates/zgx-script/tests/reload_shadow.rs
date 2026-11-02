//! What a reload would keep (#1369, phase a1): while a host watches its
//! files, each reload says what keeping the modules a save didn't change
//! would have done (the modules run again, how many kept) or why everything
//! would load again. Nothing is kept yet: every reload still loads it all.
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

/// A host of `app`, watching, loaded once more so that load is recorded.
fn watching(app: &App) -> Harness {
    let h = Harness::new(|runtime| ScriptHost::new(runtime, app.path()));
    // Watching from now on: the load before it is not recorded.
    drop(h.host.watch(Duration::from_secs(3600)));
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
    // Every reload still loads everything again (nothing is kept yet).
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
