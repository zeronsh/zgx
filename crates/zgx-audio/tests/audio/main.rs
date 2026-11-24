//! zgx-audio's tests, as one binary (see AGENTS.md): a test file goes
//! beside the others (`tests/name.rs`) and is listed here, or, when it
//! counts allocations with a global allocator, gets a `[[test]]` of its own
//! in Cargo.toml.
#[path = "../conformance.rs"]
mod conformance;
#[path = "../device.rs"]
mod device;
#[path = "../offline.rs"]
mod offline;
#[path = "../platform_exact.rs"]
mod platform_exact;
#[path = "../pulled.rs"]
mod pulled;
#[path = "../spatial.rs"]
mod spatial;
#[path = "../stock_effects.rs"]
mod stock_effects;
#[path = "../stream.rs"]
mod stream;

/// A test file nothing lists builds into nothing and its tests never run,
/// silently.
#[test]
fn every_test_file_is_listed() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let listed = std::fs::read_to_string(dir.join("audio/main.rs")).unwrap();
    let manifest = std::fs::read_to_string(dir.join("../Cargo.toml")).unwrap();
    let missing: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "rs"))
        .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
        .filter(|f| !listed.contains(&format!("\"../{f}.rs\"")))
        .filter(|f| !manifest.contains(&format!("name = \"{f}\"")))
        .collect();
    assert!(
        missing.is_empty(),
        "test files that never run: {missing:?}. List each in tests/audio/main.rs"
    );
}
