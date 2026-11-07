#![allow(dead_code)]
use serde_json::Value;
use std::path::{Path, PathBuf};
use zgx::scene::Color;
use zgx_script::ScriptHost;
use zgx_video::Timing;
use zgx_video::{ScriptStage, Stage, StageOptions};

pub fn inline(script: &str, width: f32, height: f32, fps: f64) -> Stage {
    let script = script.to_owned();
    Stage::new(
        StageOptions {
            width,
            height,
            fps,
            scale: 1.,
            background: Color(0, 0, 0, 255),
        },
        move |setting| {
            let host = ScriptHost::inline(setting.runtime, "app", script);
            host.lint_strip(false);
            host.reload()?;
            Ok(host.view())
        },
    )
    .unwrap()
}
pub fn script_stage(dir: &Path, props: Value) -> ScriptStage {
    ScriptStage::new(
        dir,
        None,
        &props,
        1.,
        Color(0, 0, 0, 255),
        Timing::default(),
    )
    .unwrap_or_else(|error| panic!("{error}"))
}
pub fn hash(pixels: &[u8]) -> u64 {
    use std::hash::{DefaultHasher, Hasher};
    let mut hasher = DefaultHasher::new();
    hasher.write(pixels);
    hasher.finish()
}
pub fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = (y * width + x) as usize * 4;
    pixels[at..at + 4].try_into().unwrap()
}
/// A new directory of its own, for a test's scripts and files.
pub fn temp_dir(name: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    // What earlier runs left: a test binary keeps its own until it exits.
    static SWEPT: std::sync::Once = std::sync::Once::new();
    SWEPT.call_once(|| {
        let mine = format!("zgx-video-test-{}-", std::process::id());
        for entry in std::fs::read_dir(std::env::temp_dir())
            .into_iter()
            .flatten()
            .flatten()
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("zgx-video-test-") && !name.starts_with(&mine) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    });
    let next = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "zgx-video-test-{}-{name}-{next}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
/// Two frames of a hundred ms: red, then blue.
pub fn write_gif(path: &Path) {
    let file = std::fs::File::create(path).unwrap();
    let palette = [255, 0, 0, 0, 0, 255];
    let mut encoder = gif::Encoder::new(file, 4, 4, &palette).unwrap();
    encoder.set_repeat(gif::Repeat::Infinite).unwrap();
    for color in [0u8, 1] {
        let mut frame = gif::Frame::from_indexed_pixels(4, 4, vec![color; 16], None);
        frame.delay = 10;
        encoder.write_frame(&frame).unwrap();
    }
}
/// Four red pixels.
pub fn write_png(path: &Path) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, 2, 2);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&[255, 0, 0, 255].repeat(4))
        .unwrap();
}
