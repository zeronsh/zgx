//! In the repository zgx-desktop is built with the repository's winit and
//! AccessKit consumer, and knows it (`zgx_winit_patched`,
//! `zgx_accesskit_patched`; build.rs): Cargo gives their `links` metadata
//! only to a crate depending on them itself, and without it everything gated
//! compiles out, silently (as zgx-gpu's `patched_wgpu_hal` test says).
#[test]
fn the_repositorys_winit_and_accesskit_are_seen() {
    // From crates.io, as published, there are no patched ones to see.
    let vendor = concat!(env!("CARGO_MANIFEST_DIR"), "/../../vendor/winit");
    if !std::path::Path::new(vendor).exists() {
        return;
    }
    if !cfg!(zgx_winit_patched) {
        panic!("zgx_winit_patched isn't set");
    }
    if !cfg!(zgx_accesskit_patched) {
        panic!("zgx_accesskit_patched isn't set");
    }
}
