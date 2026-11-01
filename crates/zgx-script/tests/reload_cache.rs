//! A hot-reloading host compiles each script file once while its bytes stay
//! the same: a reload compiles what changed (every module still runs again),
//! a save that changes no bytes reloads nothing, and what it keeps is the
//! last load's files alone.
mod common;

use common::Harness;
use std::{cell::Cell, path::PathBuf, time::Duration};
use zgx_script::ScriptHost;

/// An app requiring `modules`, each returning a label, in a directory of its
/// own; a state counts the loads.
struct App {
    dir: PathBuf,
    /// Seconds past now that the next write is stamped: each later than the
    /// last, as saves are.
    later: Cell<u64>,
}
impl App {
    fn new(name: &str, modules: &[&str]) -> Self {
        let dir =
            std::env::temp_dir().join(format!("zgx-reload-cache-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let app = Self {
            dir,
            later: Cell::new(5),
        };
        for module in modules {
            app.write(
                &format!("{module}.luau"),
                &format!("return \"{module} v1\"\n"),
            );
        }
        app.write("app.luau", &Self::main(modules));
        app
    }
    fn main(modules: &[&str]) -> String {
        let mut code = String::from(
            "local loads = ui.state(\"loads\", 0)\nloads:set(loads:peek() + 1)\nlocal labels = {}\n",
        );
        for module in modules {
            code += &format!("table.insert(labels, require(\"./{module}\"))\n");
        }
        code += "return ui.column {\n    ui.text { id = \"labels\", table.concat(labels, \",\") },\n    \
                 ui.text { id = \"loads\", \"{loads}\" },\n}\n";
        code
    }
    fn path(&self) -> PathBuf {
        self.dir.join("app.luau")
    }
    /// Writes `file`, stamped later than any write before it.
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

#[test]
fn a_reload_compiles_only_the_files_that_changed() {
    let app = App::new("changed", &["a", "b", "c", "d", "e"]);
    let mut h = Harness::new(|runtime| ScriptHost::new(runtime, app.path()));
    assert_eq!(h.text("labels"), "a v1,b v1,c v1,d v1,e v1");
    assert_eq!(
        h.host.compiled_sources(),
        (0, 0),
        "a host that loaded once keeps nothing"
    );
    // The first reload compiles every file, as before, and keeps them.
    app.write("a.luau", "return \"a v2\"\n");
    assert!(h.host.reload_if_changed());
    h.settle();
    assert_eq!(h.text("labels"), "a v2,b v1,c v1,d v1,e v1");
    assert_eq!(h.host.compiled_sources(), (6, 6));
    // The next compiles the one that changed; every module still ran again.
    app.write("c.luau", "return \"c v2\"\n");
    assert!(h.host.reload_if_changed());
    h.settle();
    assert_eq!(h.text("labels"), "a v2,b v1,c v2,d v1,e v1");
    assert_eq!(h.host.compiled_sources(), (6, 7));
    assert_eq!(h.text("loads"), "3", "each reload ran the app again");
    // A file changed back compiles again, by its bytes, not its time.
    app.write("c.luau", "return \"c v1\"\n");
    assert!(h.host.reload_if_changed());
    h.settle();
    assert_eq!(h.text("labels"), "a v2,b v1,c v1,d v1,e v1");
    assert_eq!(h.host.compiled_sources(), (6, 8));
}

#[test]
fn a_save_that_changes_no_bytes_reloads_nothing() {
    let app = App::new("same", &["a", "b"]);
    let mut h = Harness::new(|runtime| ScriptHost::new(runtime, app.path()));
    app.write("a.luau", "return \"a v2\"\n");
    assert!(h.host.reload_if_changed());
    h.settle();
    assert_eq!(h.text("loads"), "2");
    // The same bytes, a later time (an editor saving unchanged text, a
    // checkout touching files): no reload, so the app keeps its state.
    app.write("b.luau", "return \"b v1\"\n");
    app.write("app.luau", &App::main(&["a", "b"]));
    assert!(!h.host.reload_if_changed(), "nothing changed but times");
    h.settle();
    assert_eq!(h.text("loads"), "2", "the app didn't run again");
    assert_eq!(h.host.compiled_sources(), (3, 3));
    assert!(
        !h.host.reload_if_changed(),
        "and its times are taken as they are now"
    );
    // A real change after it still reloads.
    app.write("b.luau", "return \"b v2\"\n");
    assert!(h.host.reload_if_changed());
    h.settle();
    assert_eq!(h.text("labels"), "a v2,b v2");
    assert_eq!(h.text("loads"), "3");
}

#[test]
fn what_it_keeps_is_the_last_loads_files_alone() {
    let app = App::new("bounded", &["a", "b", "c"]);
    let mut h = Harness::new(|runtime| ScriptHost::new(runtime, app.path()));
    for n in 2..22 {
        app.write("a.luau", &format!("return \"a v{n}\"\n"));
        assert!(h.host.reload_if_changed());
        h.settle();
        assert_eq!(
            h.host.compiled_sources().0,
            4,
            "one entry a file, however many edits"
        );
    }
    assert_eq!(h.text("labels"), "a v21,b v1,c v1");
    // A module no longer required goes.
    app.write("app.luau", &App::main(&["a", "b"]));
    assert!(h.host.reload_if_changed());
    h.settle();
    assert_eq!(h.text("labels"), "a v21,b v1");
    assert_eq!(h.host.compiled_sources().0, 3);
}

#[test]
fn errors_read_as_they_did_through_compiled_files() {
    let app = App::new("errors", &["a", "b"]);
    let mut h = Harness::new(|runtime| ScriptHost::new(runtime, app.path()));
    app.write("a.luau", "return \"a v2\"\n");
    assert!(h.host.reload_if_changed());
    h.settle();
    // A file that doesn't compile is reported as loading its text reports it.
    app.write("b.luau", "return (\n");
    assert!(h.host.reload_if_changed());
    let error = h.host.error().get().expect("the syntax error is shown");
    assert!(error.contains("b.luau:"), "{error}");
    // One that runs into an error names its file and line from the bytecode.
    app.write("b.luau", "local x = 1\nerror(\"boom\")\n");
    assert!(h.host.reload_if_changed());
    let error = h.host.error().get().expect("the error is shown");
    assert!(
        error.contains("b.luau:2") && error.contains("boom"),
        "{error}"
    );
    // Fixed, it loads.
    app.write("b.luau", "return \"b v2\"\n");
    assert!(h.host.reload_if_changed());
    h.settle();
    assert_eq!(h.host.error().get(), None);
    assert_eq!(h.text("labels"), "a v2,b v2");
}

/// What one host compiled, another on the thread loading the same bytes
/// (at other paths) takes as it is; once no host holds it, it's gone.
#[test]
fn hosts_share_what_they_compiled_while_one_holds_it() {
    let first = App::new("shared-1", &["a", "b"]);
    let second = App::new("shared-2", &["a", "b"]);
    let one = Harness::new(|runtime| ScriptHost::new(runtime, first.path()));
    let mut two = Harness::new(|runtime| ScriptHost::new(runtime, second.path()));
    first.write("a.luau", "return \"a v2\"\n");
    assert!(one.host.reload_if_changed());
    one.settle();
    assert_eq!(one.host.compiled_sources(), (3, 3));
    second.write("a.luau", "return \"a v2\"\n");
    assert!(two.host.reload_if_changed());
    two.settle();
    assert_eq!(two.text("labels"), "a v2,b v1");
    assert_eq!(
        two.host.compiled_sources(),
        (3, 0),
        "the same three sources, compiled by the first"
    );
    drop((one, two));
    let third = App::new("shared-3", &["a", "b"]);
    let three = Harness::new(|runtime| ScriptHost::new(runtime, third.path()));
    third.write("a.luau", "return \"a v2\"\n");
    assert!(three.host.reload_if_changed());
    three.settle();
    assert_eq!(
        three.host.compiled_sources(),
        (3, 3),
        "no host held them: compiled again"
    );
}
