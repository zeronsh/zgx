//! A composition played live in an iOS app, its clips decoded by AVFoundation:
//! `scripts/ios/sim.py run -p zgx-video --example ios_player --resource
//! crates/zgx-video/examples/ios_player` (with a `clip.mp4` in its `film`
//! folder). The bundle's folder holding an `app.luau` runs, and its `film`
//! folder plays as `host:film`. `ZGX_APP_ARGS`: the script's `ui.args`.
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
    let args = zgx::env::var("ZGX_APP_ARGS")
        .map(|args| args.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default();
    let film = app.join("film");
    zgx_run::run_hosting(&app, args, move |host| {
        match zgx_video::Player::new(&zgx::reactive::Runtime::new(), &film, 2.) {
            Ok(player) => {
                player.expose(host, "film");
                // The player lives as long as the app.
                std::mem::forget(player);
            }
            Err(error) => eprintln!("FILM couldn't open: {error}"),
        }
    })
}
