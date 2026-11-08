//! A package's modules compile once for every host that requires them
//! (`ScriptHost::package`): a host made after another, or beside it, loads
//! the bytecode. An entry stays while a host holds it, and one no host
//! holds is released as a host goes idle (`zgx::idle::trim`), not before.
//! Its own process: what is kept is the process's.
use std::path::PathBuf;
use zgx::reactive::Runtime;
use zgx_script::ScriptHost;

/// A package of two modules, and an app requiring one of them (which
/// requires the other).
fn files() -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("zgx-package-bytecode-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, code) in [
        (
            "kit/button.luau",
            "local theme = require(\"./theme\")\nreturn function(label) return ui.text { id = \"b\", label .. theme.accent } end",
        ),
        ("kit/theme.luau", "return { accent = \"!\" }"),
        ("app/app.luau", "return require(\"@kit/button\")(\"Save\")"),
    ] {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, code).unwrap();
    }
    (dir.join("app/app.luau"), dir.join("kit"))
}

fn host(runtime: &Runtime, app: &PathBuf, kit: &PathBuf) -> ScriptHost {
    let host = ScriptHost::new(runtime, app).package("kit", kit);
    host.reload().unwrap();
    host
}

#[test]
fn a_packages_modules_compile_once_for_every_host_and_go_at_rest() {
    let (app, kit) = files();
    let runtime = Runtime::new();
    // The first host compiles the package's two modules; one beside it
    // finds them compiled.
    let first = host(&runtime, &app, &kit);
    assert_eq!(first.package_compiles(), (2, 0), "compiled, found");
    assert_eq!(zgx_script::packages_kept(), 2);
    let beside = host(&runtime, &app, &kit);
    assert_eq!(beside.package_compiles(), (0, 2));
    // Held by a live host, an idle trim keeps them.
    drop(first);
    zgx::idle::trim(&runtime);
    assert_eq!(zgx_script::packages_kept(), 2, "released while held");
    // No host holds them: kept until a host goes idle, so a host made
    // right after (a video's next stage) still finds them.
    drop(beside);
    assert_eq!(zgx_script::packages_kept(), 2, "released before a trim");
    let next = host(&runtime, &app, &kit);
    assert_eq!(next.package_compiles(), (0, 2));
    // At rest with no host holding them, the trim releases them, and a
    // host after compiles them again.
    drop(next);
    let idle = ScriptHost::inline(&runtime, "app", "return ui.text { \"idle\" }");
    idle.reload().unwrap();
    zgx::idle::trim(&runtime);
    assert_eq!(zgx_script::packages_kept(), 0, "kept at rest");
    let after = host(&runtime, &app, &kit);
    assert_eq!(after.package_compiles(), (2, 0));
    // A module whose bytes changed compiles anew; the old entry goes once
    // no host holds it.
    std::fs::write(kit.join("theme.luau"), "return { accent = \"?\" }").unwrap();
    after.reload().unwrap();
    assert_eq!(after.package_compiles(), (3, 1));
    zgx::idle::trim(&runtime);
    assert_eq!(zgx_script::packages_kept(), 2, "the old theme stayed");
}
