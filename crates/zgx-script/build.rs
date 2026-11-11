//! The Luau definitions this build embeds (`definitions.rs`), and whether
//! Luau was built with its asserts, as luau0-src decides it: on
//! when its build script has debug assertions (this one is built alike),
//! or unless `LUAU_CXXFLAGS` defines NDEBUG. A release build with them runs
//! scripts about a fifth slower, so it warns (`luau_asserts`).
#[path = "src/definitions.rs"]
mod definitions;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    // The Luau definitions this build embeds (`check::DEFINITIONS`): the
    // committed file without the parts of features it's built without.
    println!("cargo::rerun-if-changed=src/definitions.rs");
    println!("cargo::rerun-if-changed=types/zgx.d.luau");
    let full = std::fs::read_to_string("types/zgx.d.luau").expect("types/zgx.d.luau");
    let kept = definitions::filtered(&full, |feature| {
        let var = format!("CARGO_FEATURE_{}", feature.to_uppercase().replace('-', "_"));
        std::env::var_os(var).is_some()
    });
    let out =
        std::path::Path::new(&std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("zgx.d.luau");
    std::fs::write(out, kept).expect("the definitions written");
    println!("cargo::rerun-if-env-changed=LUAU_CXXFLAGS");
    println!("cargo::rustc-check-cfg=cfg(luau_asserts)");
    let flags = std::env::var("LUAU_CXXFLAGS").unwrap_or_default();
    let ndebug = flags.split_whitespace().any(|f| {
        let f = f.trim_start_matches(['-', '/']);
        f == "DNDEBUG" || f.starts_with("DNDEBUG=")
    });
    if cfg!(debug_assertions) || !ndebug {
        println!("cargo::rustc-cfg=luau_asserts");
    }
}
