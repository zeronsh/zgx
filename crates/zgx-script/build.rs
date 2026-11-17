//! The commit it was built from (`BUILD_REV`, `build_rev.rs`).
#[path = "src/build_rev.rs"]
mod build_rev;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    // The commit these sources are (`BUILD_REV`), read again when git's
    // files for it change.
    println!("cargo::rerun-if-changed=src/build_rev.rs");
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let found = build_rev::find(std::path::Path::new(&manifest));
    for path in &found.watched {
        println!("cargo::rerun-if-changed={}", path.display());
    }
    println!(
        "cargo::rustc-env=ZGX_BUILD_REV={}",
        found.rev.as_deref().unwrap_or("unknown")
    );
}
