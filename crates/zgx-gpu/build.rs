//! `zgx_wgpu_hal_patched` when wgpu-hal is the repository's (vendor/wgpu-hal,
//! its `links` metadata): on Windows, the preferred Direct3D 12 adapter is
//! exposed without opening a device on every other one, which that wgpu-hal
//! adds and crates.io's does not (wgpu's `request_adapter` then ranks them).
//!
//! `own_surfaces` where a window can present through surfaces of the
//! renderer's own, which keep what they held (`LayerPresent`): macOS
//! (layer_present.rs) and Windows (dxgi_present.rs).
//!
//! `d3d11` on Windows with the `d3d11` feature: frames drawn with Direct3D
//! 11 under `ZGX_D3D11=1` (d3d11_frame.rs).
//!
//! `zgx_metallib` on macOS where Xcode's Metal toolchain is: the direct
//! Metal path's first-frame shaders compiled here (`metallib`).
fn main() {
    println!("cargo::rustc-check-cfg=cfg(zgx_wgpu_hal_patched)");
    if std::env::var_os("DEP_ZGX_WGPU_HAL_PATCHED").is_some() {
        println!("cargo::rustc-cfg=zgx_wgpu_hal_patched");
    }
    println!("cargo::rustc-check-cfg=cfg(own_surfaces)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "macos" || os == "windows" {
        println!("cargo::rustc-cfg=own_surfaces");
    }
    // `d3d11`: Windows with the `d3d11` feature (d3d11_frame.rs).
    println!("cargo::rustc-check-cfg=cfg(d3d11)");
    if os == "windows" && std::env::var_os("CARGO_FEATURE_D3D11").is_some() {
        println!("cargo::rustc-cfg=d3d11");
    }
    metallib(&os);
}

/// `zgx_metallib`: the direct Metal path's two shader libraries every
/// window's first frame needs (draw.metal, copy_offset.metal) compiled
/// here, so a launch whose shader cache misses (the first after an install
/// or a system update) loads them rather than compiling them, about 150 ms
/// before its first frame on an M5. Built for macOS by a macOS host with
/// Xcode's Metal toolchain; without it (Linux, a cross build, no toolchain
/// installed) nothing is built and the shaders compile at start as before,
/// so the build never fails for it. `ZGX_NO_METALLIB` leaves them out.
/// Each library's source is hashed as compiled (`metal_frame` loads one only
/// while its source still hashes the same).
fn metallib(os: &str) {
    println!("cargo::rustc-check-cfg=cfg(zgx_metallib)");
    println!("cargo::rerun-if-changed=src/draw.metal");
    println!("cargo::rerun-if-changed=src/copy_offset.metal");
    println!("cargo::rerun-if-env-changed=ZGX_NO_METALLIB");
    println!("cargo::rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    if os != "macos" || !cfg!(target_os = "macos") || std::env::var_os("ZGX_NO_METALLIB").is_some()
    {
        return;
    }
    let Some(out) = std::env::var_os("OUT_DIR").map(std::path::PathBuf::from) else {
        return;
    };
    // MSL 2.4 (the sources' own) needs macOS 12; a later deployment target
    // is honoured. On a system it can't load on, the shaders compile at start.
    let target = std::env::var("MACOSX_DEPLOYMENT_TARGET")
        .ok()
        .filter(|version| {
            version
                .split('.')
                .next()
                .and_then(|major| major.parse::<u32>().ok())
                >= Some(12)
        })
        .unwrap_or_else(|| "12.0".to_owned());
    let min = format!("-mmacosx-version-min={target}");
    let ran = |args: &[&std::ffi::OsStr]| {
        std::process::Command::new("xcrun")
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    let mut hashes = String::new();
    for name in ["draw", "copy_offset"] {
        let source = format!("src/{name}.metal");
        let (air, lib) = (
            out.join(format!("{name}.air")),
            out.join(format!("{name}.metallib")),
        );
        let Ok(bytes) = std::fs::read(&source) else {
            return;
        };
        let compiled = ran(&[
            "-sdk".as_ref(),
            "macosx".as_ref(),
            "metal".as_ref(),
            "-std=macos-metal2.4".as_ref(),
            min.as_ref(),
            "-c".as_ref(),
            source.as_ref(),
            "-o".as_ref(),
            air.as_os_str(),
        ]) && ran(&[
            "-sdk".as_ref(),
            "macosx".as_ref(),
            "metallib".as_ref(),
            air.as_os_str(),
            "-o".as_ref(),
            lib.as_os_str(),
        ]);
        if !compiled {
            return;
        }
        hashes += &format!(
            "pub(crate) const {}_SOURCE: u64 = {:#018x};\n",
            name.to_uppercase(),
            fnv1a(&bytes)
        );
    }
    if std::fs::write(out.join("metallib_sources.rs"), hashes).is_ok() {
        println!("cargo::rustc-cfg=zgx_metallib");
    }
}

/// FNV-1a over `bytes`, as `metal_frame::fnv1a` computes it.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
    }
    hash
}
