//! The shipped example apps load and run headless without a script error:
//! what `zgx check` passes also runs (`check.rs` checks their types).
mod common;

use common::Harness;
use zgx_script::ScriptHost;

fn runs(app: &str, beside: Option<&str>) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let path = dir.join(app);
    let beside = beside.map(|p| dir.join(p));
    let h = Harness::new(move |runtime| {
        let host = ScriptHost::new(runtime, path);
        // What hot_reload.rs publishes for its scripts.
        host.shared("window", "400 × 300");
        host.action("quit", |_| {});
        match beside {
            Some(dir) => host.scripts_in(dir, zgx_script::Limits::default()),
            None => host,
        }
    });
    for _ in 0..30 {
        h.frame();
    }
    assert_eq!(h.host.error().get(), None, "{app}");
}

#[test]
fn hot_reload_runs_with_its_scripts() {
    runs("hot_reload/app.luau", Some("hot_reload/plugins"));
}
#[test]
fn notes_runs() {
    runs("notes/app.luau", None);
}
#[test]
fn web_view_runs() {
    runs("web_view/app.luau", None);
}
#[test]
fn anime_showcase_runs() {
    runs("anime_showcase/app.luau", None);
}
#[test]
fn stream_runs() {
    runs("stream/app.luau", None);
}

#[cfg(feature = "scene3d")]
#[test]
fn scene3d() {
    runs("scene3d/app.luau", None);
}
