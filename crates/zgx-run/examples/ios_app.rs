//! A Luau app inside an iOS app bundle: `scripts/ios/sim.py run -p zgx-run
//! --example ios_app --resource <app dir>`. The first folder of the bundle
//! holding an `app.luau` runs, in Luau's interpreter: iOS allows no JIT.
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bundle = std::env::current_exe()?
        .parent()
        .map(PathBuf::from)
        .ok_or("no bundle")?;
    let app = std::fs::read_dir(&bundle)?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join("app.luau").is_file())
        .ok_or("no folder in the bundle holds an app.luau")?;
    zgx_run::run(&app)
}
