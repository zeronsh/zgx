//! Files macOS writes (AVFoundation, on VideoToolbox), through the command as
//! people run it: what it chose and why, the streams' tags, the frames held
//! to the PNGs of the same render and to what ffmpeg writes of them, and the
//! no-ffmpeg path (`ZGX_VIDEO_FFMPEG=none`).
#![cfg(target_os = "macos")]
mod common;
use common::*;
use std::path::Path;
use std::process::{Command, Output};

const SWEEP: &str = r##"
local video = require("@video")
return video.compositions { video.composition {
    id = "S", w = 128, h = 72, fps = 30, duration = 20,
    render = function()
        local f = video.frame()
        return ui.div { w_full = true, h_full = true, bg = "#203040",
            ui.div { w = 48, h = 32, rounded = 8, bg = "#e0a040", x = f:map({ 0, 19 }, { 4, 76 }), y = 20 },
            ui.div { w = 24, h = 24, rounded = 12, bg = "#40c080", x = 52, y = f:map({ 0, 19 }, { 44, 4 }) },
        }
    end,
} }
"##;
const W: usize = 128;
const H: usize = 72;

fn app(name: &str) -> TempDir {
    let dir = temp_dir(name);
    std::fs::write(dir.join("video.luau"), SWEEP).unwrap();
    dir
}
/// `zgx-video render dir out args`, with `env` set.
fn render(dir: &Path, out: &str, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_zgx-video"));
    command.arg("render").arg(dir).arg(dir.join(out)).args(args);
    command
        .env_remove("ZGX_VIDEO_NATIVE")
        .env_remove("ZGX_VIDEO_FFMPEG");
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().unwrap()
}
fn said(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
fn ffmpeg() -> Option<std::path::PathBuf> {
    zgx_video::encode::ffmpeg()
}
/// What ffprobe says of the video stream.
fn probe(path: &Path, entries: &str) -> String {
    let ffprobe = ffmpeg().unwrap().with_file_name("ffprobe");
    let out = Command::new(ffprobe)
        .args(["-v", "error", "-select_streams", "v", "-show_entries"])
        .arg(format!("stream={entries}"))
        .args(["-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}
/// Every frame of `path` as RGB, decoded by ffmpeg.
fn decoded(path: &Path) -> Vec<u8> {
    let out = Command::new(ffmpeg().unwrap())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", said(&out));
    out.stdout
}
/// The PNG frames in `dir`, as RGB, one after another.
fn png_frames(dir: &Path) -> Vec<u8> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    names.sort();
    let mut rgb = Vec::new();
    for name in names {
        let file = std::fs::File::open(name).unwrap();
        let mut reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        rgb.extend(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2]]),
        );
    }
    rgb
}
fn psnr(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len(), "frame counts differ");
    let squared: f64 = a
        .iter()
        .zip(b)
        .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
        .sum();
    let mse = squared / a.len() as f64;
    if mse == 0. {
        return f64::INFINITY;
    }
    10. * (255f64 * 255. / mse).log10()
}

#[test]
fn macos_writes_h264_hevc_and_prores_as_well_tagged_and_near_ffmpeg() {
    if ffmpeg().is_none() {
        eprintln!("no ffmpeg here to read them with: skipped");
        return;
    }
    let dir = app("formats");
    assert!(render(&dir, "frames", &["--quiet"], &[]).status.success());
    let reference = png_frames(&dir.join("frames"));
    assert_eq!(reference.len(), W * H * 3 * 20);
    // Sharp coloured edges at 128×72 lose most to 4:2:0: libx264 itself
    // measures near 36 dB here. Each is held to that, and to ffmpeg's own.
    let mut measured = Vec::new();
    for (codec, out, tags, floor) in [
        (
            "h264",
            "native.mp4",
            "h264,yuv420p,tv,bt709,bt709,bt709,20",
            32.,
        ),
        (
            "hevc",
            "native-hevc.mp4",
            "hevc,yuv420p,tv,bt709,bt709,bt709,20",
            32.,
        ),
        (
            "prores",
            "native.mov",
            "prores,yuva444p12le,tv,bt709,bt709,bt709,20",
            40.,
        ),
    ] {
        // Four chunks asked for: a file macOS writes is written whole.
        let native = render(
            &dir,
            out,
            &["--codec", codec, "--concurrency", "4"],
            &[("ZGX_VIDEO_NATIVE", "1")],
        );
        assert!(native.status.success(), "{codec}: {}", said(&native));
        assert!(
            said(&native).contains("by macOS (ZGX_VIDEO_NATIVE)"),
            "{codec}: {}",
            said(&native)
        );
        let file = dir.join(out);
        assert_eq!(
            probe(
                &file,
                "codec_name,pix_fmt,color_range,color_space,color_transfer,color_primaries,nb_frames"
            ),
            tags,
            "{codec}"
        );
        let theirs = format!("ffmpeg-{out}");
        let by_ffmpeg = render(&dir, &theirs, &["--codec", codec, "--quiet"], &[]);
        assert!(by_ffmpeg.status.success(), "{codec}: {}", said(&by_ffmpeg));
        let (native, by_ffmpeg) = (
            psnr(&decoded(&file), &reference),
            psnr(&decoded(&dir.join(&theirs)), &reference),
        );
        eprintln!("{codec}: macOS {native:.2} dB, ffmpeg {by_ffmpeg:.2} dB");
        measured.push((codec, native, by_ffmpeg, floor));
    }
    for (codec, native, by_ffmpeg, floor) in measured {
        assert!(
            native >= floor && native >= by_ffmpeg - 3.,
            "{codec}: macOS {native:.2} dB, ffmpeg {by_ffmpeg:.2} dB"
        );
    }
}

#[test]
fn the_command_says_who_writes_the_file_and_why() {
    let dir = app("says");
    let out = render(&dir, "out.mp4", &[], &[]);
    assert!(out.status.success(), "{}", said(&out));
    if ffmpeg().is_some_and(|_| {
        zgx_video::encode::choice(zgx_video::Format::H264).0 == zgx_video::encode::Writer::Ffmpeg
    }) {
        assert!(
            said(&out).contains("H.264 by ffmpeg's libx264"),
            "{}",
            said(&out)
        );
    }
    let none = render(&dir, "none.mp4", &[], &[("ZGX_VIDEO_FFMPEG", "none")]);
    assert!(none.status.success(), "{}", said(&none));
    assert!(
        said(&none).contains("H.264 by macOS, as no ffmpeg was found")
            && said(&none).contains("53.5 dB"),
        "{}",
        said(&none)
    );
}

#[test]
fn without_ffmpeg_macos_writes_the_whole_file_in_one_piece() {
    let dir = app("whole");
    // Chunks on threads would need ffmpeg to join them.
    let out = render(
        &dir,
        "out.mp4",
        &["--concurrency", "4", "--quiet"],
        &[("ZGX_VIDEO_FFMPEG", "none")],
    );
    assert!(out.status.success(), "{}", said(&out));
    let file = dir.join("out.mp4");
    assert!(std::fs::metadata(&file).unwrap().len() > 0);
    if ffmpeg().is_some() {
        assert_eq!(probe(&file, "codec_name,nb_frames"), "h264,20");
        assert_eq!(decoded(&file).len(), W * H * 3 * 20);
    }
}

#[test]
fn without_ffmpeg_sound_or_vp9_says_how_to_get_a_file() {
    let dir = app("needs");
    let tone: Vec<f32> = (0..48000 * 2)
        .map(|i| (i as f32 * 0.02).sin() * 0.3)
        .collect();
    zgx_video::audio::write_wav(&dir.join("a.wav"), &tone).unwrap();
    std::fs::write(
        dir.join("video.luau"),
        SWEEP.replace(
            r##"bg = "#203040","##,
            r##"bg = "#203040", video.audio { src = "a.wav" },"##,
        ),
    )
    .unwrap();
    let none = [("ZGX_VIDEO_FFMPEG", "none")];
    let sound = render(&dir, "out.mp4", &["--quiet"], &none);
    assert!(!sound.status.success());
    assert!(
        said(&sound).contains("needs ffmpeg to add this video's sound")
            && said(&sound).contains("--mute"),
        "{}",
        said(&sound)
    );
    let muted = render(&dir, "muted.mp4", &["--mute", "--quiet"], &none);
    assert!(muted.status.success(), "{}", said(&muted));
    let vp9 = render(&dir, "out.webm", &["--mute", "--quiet"], &none);
    assert!(!vp9.status.success());
    assert!(
        said(&vp9).contains("found none on the PATH") && said(&vp9).contains("which macOS writes"),
        "{}",
        said(&vp9)
    );
}
