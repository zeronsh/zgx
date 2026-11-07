//! Video files in a video: each frame shows the file's frame its time says,
//! played on, rendered as a range, or seeked back to.
mod common;
use common::*;
use serde_json::json;
use std::io::Write;

/// A 30 fps file of three colours, ten frames each: red, green, blue.
fn write_clip(path: &std::path::Path) -> bool {
    let Some(ffmpeg) = zgx_video::encode::ffmpeg() else {
        return false;
    };
    let mut child = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo"])
        .args(["-pix_fmt", "rgb24", "-s", "16x16", "-r", "30", "-i", "-"])
        .args(["-c:v", "libx264rgb", "-qp", "0"])
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.as_mut().unwrap();
    for color in [[255u8, 0, 0], [0, 255, 0], [0, 0, 255]] {
        for _ in 0..10 {
            stdin.write_all(&color.repeat(16 * 16)).unwrap();
        }
    }
    drop(child.stdin.take());
    child.wait().unwrap().success()
}
fn app(name: &str) -> Option<std::path::PathBuf> {
    let dir = temp_dir(name);
    if !write_clip(&dir.join("three.mp4")) {
        eprintln!("no ffmpeg with libx264rgb here: skipped");
        return None;
    }
    std::fs::write(
        dir.join("video.luau"),
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "C", w = 20, h = 20, fps = 30, duration = 40,
            render = function()
                return ui.div { w_full = true, h_full = true, bg = "#000000",
                    video.clip { src = "three.mp4", from = 5, trim = 10, w = 20, h = 20 } }
            end,
        } }
        "##,
    )
    .unwrap();
    Some(dir)
}
fn color(pixels: &[u8]) -> &'static str {
    match pixel(pixels, 20, 10, 10) {
        [r, g, b, _] if r > 200 && g < 50 && b < 50 => "red",
        [r, g, b, _] if g > 200 && r < 50 && b < 50 => "green",
        [r, g, b, _] if b > 200 && r < 50 && g < 50 => "blue",
        [r, g, b, _] if r < 20 && g < 20 && b < 20 => "none",
        _ => "other",
    }
}

#[test]
fn a_clip_shows_the_frame_its_time_says() {
    let Some(dir) = app("plays") else { return };
    let mut stage = script_stage(&dir, json!({}));
    let mut pixels = Vec::new();
    let mut seen = Vec::new();
    for frame in [0, 4, 5, 14, 15, 24, 39] {
        stage.step_to(frame).unwrap();
        stage.draw(&mut pixels).unwrap();
        seen.push(color(&pixels));
    }
    // From frame 5, ten frames in: green, then blue, and its last held.
    assert_eq!(
        seen,
        ["none", "none", "green", "green", "blue", "blue", "blue"]
    );
}

#[test]
fn a_clip_seeked_back_to_shows_that_frame() {
    let Some(dir) = app("seek") else { return };
    let mut pixels = Vec::new();
    let mut stage = script_stage(&dir, json!({}));
    stage.step_to(20).unwrap();
    stage.draw(&mut pixels).unwrap();
    assert_eq!(color(&pixels), "blue");
    drop(stage);
    // A render of frames 6 to 7 alone starts there.
    let out = dir.join("range");
    zgx_video::render(
        &zgx_video::RenderOptions {
            script: dir.clone(),
            output: out.clone(),
            frames: Some((6, 7)),
            ..Default::default()
        },
        |_| true,
    )
    .unwrap();
    let file = std::fs::File::open(out.join("00000.png")).unwrap();
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .unwrap();
    let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut decoded).unwrap();
    assert_eq!(color(&decoded), "green");
}
