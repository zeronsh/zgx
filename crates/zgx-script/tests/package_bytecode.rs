//! A package's modules compile once for every host that requires them
//! (`ScriptHost::package`): a host made after another, or beside it, loads
//! the bytecode. No host holds it: the table keeps it until a host goes
//! idle (`zgx::idle::trim`), which releases it all. A hot-reloading host
//! keeps its own. Its own process: what is kept is the process's.
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
fn a_packages_modules_compile_once_for_every_host_until_a_trim() {
    let (app, kit) = files();
    let runtime = Runtime::new();
    // The first host compiles the package's two modules; one beside it
    // finds them compiled.
    let first = host(&runtime, &app, &kit);
    assert_eq!(first.package_compiles(), (2, 0), "compiled, found");
    assert_eq!(zgx_script::packages_kept(), 2);
    let beside = host(&runtime, &app, &kit);
    assert_eq!(beside.package_compiles(), (0, 2));
    // Gone with no trim between: a host made right after (a video's next
    // stage) still finds them.
    drop((first, beside));
    let next = host(&runtime, &app, &kit);
    assert_eq!(next.package_compiles(), (0, 2));
    // A host going idle releases them all, a live host's included: it
    // holds none (its VM keeps what it loaded, and still runs).
    zgx::idle::trim(&runtime);
    assert_eq!(zgx_script::packages_kept(), 0, "kept at rest");
    assert!(next.reload().is_ok(), "a live host loads again");
    assert_eq!(
        next.package_compiles(),
        (2, 2),
        "compiled again after a trim"
    );
    // A module whose bytes changed compiles anew.
    std::fs::write(kit.join("theme.luau"), "return { accent = \"?\" }").unwrap();
    next.reload().unwrap();
    assert_eq!(next.package_compiles(), (3, 3));
    zgx::idle::trim(&runtime);
    assert_eq!(zgx_script::packages_kept(), 0);
    // A hot-reloading host keeps its packages' modules itself, by path, as
    // its other files: none in the table, so a trim between edits doesn't
    // make each reload compile them again.
    let hot = ScriptHost::new(&runtime, &app).package("kit", &kit);
    assert!(hot.reload_if_changed(), "loaded, hot-reloading");
    assert_eq!(hot.package_compiles(), (0, 0));
    assert_eq!(zgx_script::packages_kept(), 0);
}
