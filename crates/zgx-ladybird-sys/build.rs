//! With the `build` feature: build zgx's Ladybird host and Ladybird (see
//! README.md and ladybird/build.py, which does the work). Without it,
//! nothing.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=ZGX_LADYBIRD_CACHE");
    if std::env::var_os("CARGO_FEATURE_BUILD").is_none() {
        return;
    }
    for path in ["host", "ladybird"] {
        println!("cargo:rerun-if-changed={path}");
    }
    let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let python = std::env::var("PYTHON3").unwrap_or_else(|_| "python3".into());
    let output = std::process::Command::new(&python)
        .arg("-I")
        .arg(manifest.join("ladybird/build.py"))
        .arg("--print-install-dir")
        .stderr(std::process::Stdio::inherit())
        .output()
        .unwrap_or_else(|error| panic!("zgx-ladybird-sys: running {python}: {error}"));
    if !output.status.success() {
        panic!("zgx-ladybird-sys: building Ladybird failed (see above)");
    }
    let install = String::from_utf8(output.stdout).expect("a path");
    let install = install
        .lines()
        .last()
        .expect("build.py prints the install dir")
        .trim();
    println!("cargo:rustc-env=ZGX_LADYBIRD_INSTALL_DIR={install}");
}
