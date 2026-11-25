//! In the repository zgx-gpu is built with the repository's wgpu-hal, and
//! knows it: `zgx_wgpu_hal_patched` (build.rs) is set on every platform it
//! gates anything on. Cargo gives the `links` metadata it is read from only
//! to a crate depending on wgpu-hal itself, so without that dependency
//! everything gated compiles out, silently: partial presents on Linux,
//! Metal's queue rested and its shaders' IR let go on macOS.
#[test]
fn the_repositorys_wgpu_hal_is_seen() {
    // From crates.io, as published, there is no patched wgpu-hal to see.
    let vendored = concat!(env!("CARGO_MANIFEST_DIR"), "/../../vendor/wgpu-hal");
    if !std::path::Path::new(vendored).exists() {
        return;
    }
    if !cfg!(zgx_wgpu_hal_patched) {
        panic!("zgx_wgpu_hal_patched isn't set: zgx-gpu needs wgpu-hal as a dependency of its own");
    }
}
