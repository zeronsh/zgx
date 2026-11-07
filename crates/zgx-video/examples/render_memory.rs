//! Memory across renders in one process: what a host rendering video after
//! video keeps. Renders `DIR` (the showcase) `TIMES` times (3), its first
//! `FRAMES` frames (600) at `SCALE` (1), printing the resident size after each.
//! Run: cargo run --release -p zgx-video --example render_memory
fn resident_mb() -> f64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .unwrap_or(0.)
        / 1024.
}
fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
fn main() {
    let dir: String = env(
        "DIR",
        concat!(env!("CARGO_MANIFEST_DIR"), "/examples/showcase").to_owned(),
    );
    let times: usize = env("TIMES", 3);
    let frames: u64 = env("FRAMES", 600);
    let out = std::env::temp_dir().join(format!("zgx-render-memory-{}", std::process::id()));
    println!("before: {:.0} MB", resident_mb());
    for time in 0..times {
        let options = zgx_video::RenderOptions {
            script: dir.clone().into(),
            output: out.clone(),
            scale: env("SCALE", 1.),
            frames: Some((0, frames - 1)),
            mute: true,
            ..Default::default()
        };
        let mut peak = 0f64;
        let report = zgx_video::render(&options, |progress| {
            if progress.done % 100 == 0 {
                peak = peak.max(resident_mb());
            }
            true
        })
        .expect("a render");
        println!(
            "render {}: {} frames, {:.0} fps; peak {:.0} MB, after {:.0} MB",
            time + 1,
            report.frames,
            report.rate,
            peak,
            resident_mb()
        );
    }
    let _ = std::fs::remove_dir_all(&out);
}
