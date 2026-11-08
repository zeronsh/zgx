//! Files, as they come out.
mod common;
use common::*;
use serde_json::json;
use zgx_video::{Format, RenderOptions, render};

const MOVES: &str = r##"
local video = require("@video")
return video.compositions { video.composition {
    id = "M", w = 64, h = 48, fps = 30, duration = 20,
    render = function()
        local f = video.frame()
        return ui.div { w_full = true, h_full = true,
            ui.div { w = 20, h = 20, rounded = 4, bg = "#ff0000", x = f:map({ 0, 19 }, { 0, 40 }), y = f:spring { to = 20 } },
        }
    end,
} }
"##;
fn app(name: &str) -> TempDir {
    let dir = temp_dir(name);
    std::fs::write(dir.join("video.luau"), MOVES).unwrap();
    dir
}
fn pngs(dir: &std::path::Path) -> Vec<Vec<u8>> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
        .collect();
    names.sort();
    names
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect()
}
fn options(dir: &std::path::Path, out: &str) -> RenderOptions {
    RenderOptions {
        script: dir.to_owned(),
        output: dir.join(out),
        ..Default::default()
    }
}

#[test]
fn the_same_input_writes_the_same_frames() {
    let dir = app("same");
    render(&options(&dir, "a"), |_| true).unwrap();
    render(&options(&dir, "b"), |_| true).unwrap();
    let (a, b) = (pngs(&dir.join("a")), pngs(&dir.join("b")));
    assert_eq!(a.len(), 20);
    assert!(a == b, "the two renders differ");
    assert_ne!(a[0], a[19]);
}

#[test]
fn a_range_of_frames_is_those_frames_of_the_whole() {
    let dir = app("range");
    render(&options(&dir, "whole"), |_| true).unwrap();
    let mut part = options(&dir, "part");
    part.frames = Some((12, 15));
    render(&part, |_| true).unwrap();
    let (whole, part) = (pngs(&dir.join("whole")), pngs(&dir.join("part")));
    assert_eq!(part.len(), 4);
    assert!(
        part == whole[12..16],
        "a range drew other frames than the whole"
    );
}

#[test]
fn a_transparent_png_keeps_its_alpha() {
    let dir = app("alpha");
    let mut clear = options(&dir, "clear");
    clear.transparent = true;
    clear.frames = Some((0, 0));
    render(&clear, |_| true).unwrap();
    let file = std::fs::File::open(dir.join("clear/00000.png")).unwrap();
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut pixels).unwrap();
    assert_eq!(pixel(&pixels, 64, 60, 40)[3], 0, "nothing there is clear");
    assert_eq!(
        pixel(&pixels, 64, 10, 10),
        [255, 0, 0, 255],
        "the box is solid"
    );
}

#[test]
fn a_higher_frame_rate_plays_at_the_same_speed() {
    let dir = app("fps");
    let mut double = options(&dir, "double");
    double.fps = Some(60.);
    render(&double, |_| true).unwrap();
    render(&options(&dir, "single"), |_| true).unwrap();
    let (double, single) = (pngs(&dir.join("double")), pngs(&dir.join("single")));
    assert_eq!(double.len(), 40);
    // Each frame of the slower is a frame of the faster.
    assert!(double[10] == single[5] && double[24] == single[12]);
}

#[test]
fn motion_blur_spreads_a_moving_edge() {
    let dir = app("blur");
    let mut sharp = options(&dir, "sharp");
    sharp.frames = Some((10, 10));
    render(&sharp, |_| true).unwrap();
    let mut blurred = options(&dir, "blurred");
    blurred.frames = Some((10, 10));
    blurred.samples = 8;
    blurred.shutter = 1.;
    render(&blurred, |_| true).unwrap();
    let solid_or_not = |path: &str| {
        let file = std::fs::File::open(dir.join(path).join("00000.png")).unwrap();
        let mut reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        pixels.chunks(4).filter(|p| p[0] > 12 && p[0] < 240).count()
    };
    assert!(solid_or_not("blurred") > solid_or_not("sharp") * 2);
}

#[test]
fn a_gif_has_every_frame() {
    let dir = app("gif");
    let report = render(&options(&dir, "out.gif"), |_| true).unwrap();
    assert_eq!(report.frames, 20);
    let file = std::fs::File::open(dir.join("out.gif")).unwrap();
    let mut decoder = gif::DecodeOptions::new().read_info(file).unwrap();
    let mut count = 0;
    let mut delay = 0;
    while let Some(frame) = decoder.read_next_frame().unwrap() {
        count += 1;
        delay += u32::from(frame.delay);
    }
    assert_eq!(count, 20);
    assert_eq!(delay, 67, "20 frames at 30 fps, in hundredths");
}

/// What ffprobe says of a stream, or None without it.
fn probe(path: &std::path::Path, entries: &str, stream: &str) -> Option<String> {
    let ffprobe = zgx_video::encode::ffmpeg()?.with_file_name("ffprobe");
    let out = std::process::Command::new(ffprobe)
        .args(["-v", "error", "-select_streams", stream, "-show_entries"])
        .arg(format!("stream={entries}"))
        .args(["-of", "csv=p=0"])
        .arg(path)
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

#[test]
fn an_mp4_says_its_colours_and_has_its_frames() {
    if zgx_video::encode::ffmpeg().is_none() {
        eprintln!("no ffmpeg here: skipped");
        return;
    }
    let dir = app("mp4");
    render(&options(&dir, "out.mp4"), |_| true).unwrap();
    let out = dir.join("out.mp4");
    let said = probe(
        &out,
        "codec_name,pix_fmt,color_space,color_transfer,color_primaries,color_range,nb_frames",
        "v",
    )
    .unwrap();
    assert_eq!(said, "h264,yuv420p,tv,bt709,bt709,bt709,20");
}

#[test]
fn prores_keeps_its_alpha() {
    if zgx_video::encode::ffmpeg().is_none() {
        eprintln!("no ffmpeg here: skipped");
        return;
    }
    let dir = app("prores");
    let mut clear = options(&dir, "out.mov");
    clear.transparent = true;
    render(&clear, |_| true).unwrap();
    let said = probe(&dir.join("out.mov"), "codec_name,pix_fmt", "v").unwrap();
    assert!(said.starts_with("prores,yuva444p"), "{said}");
}

#[test]
fn sounds_a_script_places_are_muxed() {
    if zgx_video::encode::ffmpeg().is_none() {
        eprintln!("no ffmpeg here: skipped");
        return;
    }
    let dir = temp_dir("audio");
    let tone: Vec<f32> = (0..48000 * 2)
        .map(|i| (i as f32 * 0.03).sin() * 0.5)
        .collect();
    zgx_video::audio::write_wav(&dir.join("tone.wav"), &tone).unwrap();
    std::fs::write(
        dir.join("video.luau"),
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "A", w = 32, h = 32, fps = 30, duration = 30,
            render = function()
                return ui.div { w_full = true, h_full = true, bg = "#123",
                    video.sequence { from = 3, function()
                        return video.audio { src = "tone.wav", volume = 0.5 }
                    end },
                }
            end,
        } }
        "##,
    )
    .unwrap();
    let report = render(&options(&dir, "out.mp4"), |_| true).unwrap();
    assert_eq!(report.frames, 30);
    let said = probe(&dir.join("out.mp4"), "codec_name,sample_rate,channels", "a").unwrap();
    assert_eq!(said, "aac,48000,2");
}

#[test]
fn sound_as_long_as_the_frames_ends_with_them() {
    if zgx_video::encode::ffmpeg().is_none() {
        eprintln!("no ffmpeg here: skipped");
        return;
    }
    let dir = temp_dir("aac-end");
    let tone: Vec<f32> = (0..48000 * 2 * 2)
        .map(|i| (i as f32 * 0.03).sin() * 0.5)
        .collect();
    zgx_video::audio::write_wav(&dir.join("tone.wav"), &tone).unwrap();
    // 30 frames at 30 fps: a mix of 48,000 samples, not a whole number of
    // AAC's 1,024-sample frames, whose last is padded.
    std::fs::write(
        dir.join("video.luau"),
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "E", w = 32, h = 32, fps = 30, duration = 30,
            render = function()
                return ui.div { w_full = true, h_full = true, bg = "#543",
                    video.audio { src = "tone.wav" },
                }
            end,
        } }
        "##,
    )
    .unwrap();
    assert_eq!(
        render(&options(&dir, "out.mp4"), |_| true).unwrap().frames,
        30
    );
    let out = dir.join("out.mp4");
    assert_eq!(probe(&out, "nb_frames", "v").unwrap(), "30");
    let video: f64 = probe(&out, "duration", "v").unwrap().parse().unwrap();
    let audio: f64 = probe(&out, "duration", "a").unwrap().parse().unwrap();
    assert!(
        audio <= video + 0.001,
        "sound {audio} s past frames {video} s"
    );
}

#[test]
fn a_file_at_another_rate_keeps_every_frame_and_no_more_sound() {
    if zgx_video::encode::ffmpeg().is_none() {
        eprintln!("no ffmpeg here: skipped");
        return;
    }
    let dir = temp_dir("rate");
    let tone: Vec<f32> = (0..48000 * 2 * 2)
        .map(|i| (i as f32 * 0.03).sin() * 0.5)
        .collect();
    zgx_video::audio::write_wav(&dir.join("tone.wav"), &tone).unwrap();
    // 32 frames at 30 fps written at 24: 26 frames, 1.083 s. The mix in
    // whole frames of the composition was rounded to 33 (1.1 s), longer
    // than the frames; it's 32 now, and nothing is cut to fit.
    std::fs::write(
        dir.join("video.luau"),
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "R", w = 32, h = 32, fps = 30, duration = 32,
            render = function()
                return ui.div { w_full = true, h_full = true, bg = "#345",
                    video.audio { src = "tone.wav" },
                }
            end,
        } }
        "##,
    )
    .unwrap();
    let mut at_24 = options(&dir, "out.mp4");
    at_24.fps = Some(24.);
    assert_eq!(render(&at_24, |_| true).unwrap().frames, 26);
    let out = dir.join("out.mp4");
    assert_eq!(probe(&out, "nb_frames", "v").unwrap(), "26");
    let video: f64 = probe(&out, "duration", "v").unwrap().parse().unwrap();
    let audio: f64 = probe(&out, "duration", "a").unwrap().parse().unwrap();
    assert!(
        audio <= video + 0.001,
        "sound {audio} s past frames {video} s"
    );
}

#[test]
fn a_missing_format_says_how_to_say_one() {
    let dir = app("format");
    let error = render(&options(&dir, "out.xyz"), |_| true).unwrap_err();
    assert!(error.contains("--codec"), "{error}");
    let mut hevc = options(&dir, "h.mp4");
    hevc.format = Some(Format::Hevc);
    let _ = json!({});
}

#[test]
fn a_render_returns_what_it_took() {
    // Each stage's thread and pixels go with it: many renders in a process
    // hold no more than a few.
    let dir = app("memory");
    let render_all = |count: usize| {
        for i in 0..count {
            let mut out = options(&dir, &format!("m{i}"));
            out.frames = Some((0, 1));
            render(&out, |_| true).unwrap();
        }
    };
    render_all(4);
    let settled = resident();
    render_all(16);
    let grown = resident().saturating_sub(settled);
    assert!(grown < 24 << 20, "16 more renders kept {} MiB", grown >> 20);
}
fn resident() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
        * 1024
}

#[test]
fn hevc_is_tagged_for_players() {
    if zgx_video::encode::ffmpeg().is_none() {
        eprintln!("no ffmpeg here: skipped");
        return;
    }
    let dir = app("hevc");
    let mut hevc = options(&dir, "out.mp4");
    hevc.format = Some(Format::Hevc);
    render(&hevc, |_| true).unwrap();
    let said = probe(
        &dir.join("out.mp4"),
        "codec_name,codec_tag_string,color_primaries,nb_frames",
        "v",
    )
    .unwrap();
    assert_eq!(said, "hevc,hvc1,bt709,20");
}

#[test]
fn a_dataset_makes_a_video_of_each_row() {
    let dir = temp_dir("dataset");
    std::fs::write(
        dir.join("video.luau"),
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "D", w = 40, h = 20, fps = 10, duration = 2, props = { width = 1 },
            render = function(props)
                return ui.div { w_full = true, h_full = true, bg = "#000000",
                    ui.div { h = 20, w = props.width, bg = "#ff0000" } }
            end,
        } }
        "##,
    )
    .unwrap();
    std::fs::write(
        dir.join("rows.json"),
        r#"[{ "title": "a b", "width": 10 }, { "title": "c", "width": 30 }]"#,
    )
    .unwrap();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_zgx-video"))
        .arg("render")
        .arg(&*dir)
        .arg(dir.join("out-{title}"))
        .args(["--dataset"])
        .arg(dir.join("rows.json"))
        .arg("--quiet")
        .status()
        .unwrap();
    assert!(status.success());
    let reach = |name: &str| {
        let file = std::fs::File::open(dir.join(name).join("00000.png")).unwrap();
        let mut reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        (0..40u32)
            .filter(|x| pixel(&pixels, 40, *x, 10)[0] > 128)
            .count()
    };
    assert_eq!(reach("out-a-b"), 10);
    assert_eq!(reach("out-c"), 30);
}

#[test]
fn a_hold_never_released_stops_the_render_and_says_what() {
    let dir = temp_dir("hold");
    std::fs::write(
        dir.join("video.luau"),
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "H", w = 20, h = 20, duration = 3,
            render = function()
                video.hold("the prices")
                return ui.div {}
            end,
        } }
        "##,
    )
    .unwrap();
    let mut held = options(&dir, "frames");
    held.hold_timeout = std::time::Duration::from_millis(50);
    let error = render(&held, |_| true).unwrap_err();
    assert!(error.contains("the prices"), "{error}");
}

#[test]
fn a_rust_view_renders_as_a_script_does() {
    use zgx::compose::prelude::*;
    use zgx::scene::Color;
    use zgx_video::compose::{RustComposition, Spring, render_view, sequence};
    let dir = temp_dir("rust");
    let composition = RustComposition {
        width: 64.,
        height: 20.,
        fps: 30.,
        duration: 30,
    };
    let out = RenderOptions {
        output: dir.join("frames"),
        ..Default::default()
    };
    let report = render_view(
        &composition,
        &out,
        |_| {
            div()
                .w(64.)
                .h(20.)
                .bg(Color(0, 0, 0, 255))
                .child(sequence(10., None, |clock| {
                    div()
                        .h(20.)
                        .bg(Color(255, 0, 0, 255))
                        .reactive_style(move || {
                            let width = clock.spring(&Spring {
                                to: 60.,
                                stiffness: 200.,
                                damping: 30.,
                                ..Default::default()
                            });
                            Styles::new().w(width as f32)
                        })
                }))
        },
        |_| true,
    )
    .unwrap();
    assert_eq!(report.frames, 30);
    let frames = pngs(&dir.join("frames"));
    let reach = |png_bytes: &[u8]| {
        let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        (0..64u32)
            .filter(|x| pixel(&pixels, 64, *x, 10)[0] > 128)
            .count()
    };
    assert_eq!(reach(&frames[9]), 0, "before its sequence");
    assert!(reach(&frames[29]) >= 58, "settled by its end");
}

#[test]
fn chunks_rendered_at_once_are_the_frames_of_one_render() {
    let dir = app("chunks");
    render(&options(&dir, "serial"), |_| true).unwrap();
    let mut chunked = options(&dir, "chunked");
    chunked.concurrency = 3;
    let mut seen = 0;
    let report = render(&chunked, |progress| {
        seen = progress.done;
        true
    })
    .unwrap();
    assert_eq!((report.frames, seen), (20, 20));
    let (serial, chunked) = (pngs(&dir.join("serial")), pngs(&dir.join("chunked")));
    assert_eq!(chunked.len(), 20);
    assert!(
        serial == chunked,
        "chunks drew other frames than one render"
    );
}

#[test]
fn a_chunked_mp4_has_every_frame_and_its_sound() {
    if zgx_video::encode::ffmpeg().is_none() {
        eprintln!("no ffmpeg here: skipped");
        return;
    }
    let dir = temp_dir("chunked-mp4");
    let tone: Vec<f32> = (0..48000 * 2)
        .map(|i| (i as f32 * 0.03).sin() * 0.5)
        .collect();
    zgx_video::audio::write_wav(&dir.join("tone.wav"), &tone).unwrap();
    std::fs::write(
        dir.join("video.luau"),
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "A", w = 32, h = 32, fps = 30, duration = 45,
            render = function()
                local f = video.frame()
                return ui.div { w_full = true, h_full = true, bg = "#123",
                    ui.div { w = 8, h = 8, bg = "#f00", x = f:map({ 0, 44 }, { 0, 24 }) },
                    video.audio { src = "tone.wav" } }
            end,
        } }
        "##,
    )
    .unwrap();
    let mut chunked = options(&dir, "out.mp4");
    chunked.concurrency = 4;
    render(&chunked, |_| true).unwrap();
    let out = dir.join("out.mp4");
    assert_eq!(
        probe(&out, "codec_name,color_primaries,nb_frames", "v").unwrap(),
        "h264,bt709,45"
    );
    assert_eq!(probe(&out, "codec_name,channels", "a").unwrap(), "aac,2");
    let leftovers = std::fs::read_dir(&*dir)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".part")
        })
        .count();
    assert_eq!(leftovers, 0, "its chunks are removed");
}

#[test]
fn a_still_is_that_frame_of_a_render_and_a_thumbnail_fits_its_width() {
    let dir = app("still");
    let mut whole = options(&dir, "whole");
    whole.transparent = true;
    render(&whole, |_| true).unwrap();
    let mut still_options = options(&dir, "unused");
    still_options.transparent = true;
    let still = zgx_video::render_still(&still_options, 13, None).unwrap();
    assert_eq!((still.width, still.height), (64, 48));
    let file = std::fs::File::open(dir.join("whole/00013.png")).unwrap();
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .unwrap();
    let mut rendered = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut rendered).unwrap();
    assert!(
        still.pixels == rendered,
        "the still differs from that frame of a render"
    );
    let thumb = zgx_video::render_still(&options(&dir, "unused"), 13, Some(32)).unwrap();
    assert_eq!((thumb.width, thumb.height), (32, 24));
    still.write_png(&dir.join("s.png")).unwrap();
    assert!(dir.join("s.png").exists());
    assert!(
        zgx_video::render_still(&options(&dir, "unused"), 99, None)
            .unwrap_err()
            .contains("no frame 99")
    );
}

#[test]
fn every_nth_frame_keeps_those_frames_at_their_rate() {
    let dir = app("nth");
    render(&options(&dir, "all"), |_| true).unwrap();
    let mut nth = options(&dir, "nth");
    nth.every_nth = 3;
    let report = render(&nth, |_| true).unwrap();
    assert_eq!(report.frames, 7);
    let (all, nth) = (pngs(&dir.join("all")), pngs(&dir.join("nth")));
    assert_eq!(nth.len(), 7);
    assert!(nth[0] == all[0] && nth[2] == all[6] && nth[6] == all[18]);
    let mut gif = options(&dir, "nth.gif");
    gif.every_nth = 2;
    render(&gif, |_| true).unwrap();
    let file = std::fs::File::open(dir.join("nth.gif")).unwrap();
    let mut decoder = gif::DecodeOptions::new().read_info(file).unwrap();
    let (mut count, mut delay) = (0, 0);
    while let Some(frame) = decoder.read_next_frame().unwrap() {
        count += 1;
        delay += u32::from(frame.delay);
    }
    assert_eq!((count, delay), (10, 67), "10 frames over the same 0.67 s");
}

#[test]
fn every_nth_frame_in_chunks_is_every_nth_frame() {
    let dir = app("nth-chunks");
    let mut serial = options(&dir, "serial");
    serial.every_nth = 3;
    render(&serial, |_| true).unwrap();
    let mut chunked = options(&dir, "chunked");
    chunked.every_nth = 3;
    chunked.concurrency = 3;
    render(&chunked, |_| true).unwrap();
    let (serial, chunked) = (pngs(&dir.join("serial")), pngs(&dir.join("chunked")));
    assert_eq!(chunked.len(), 7);
    assert!(
        serial == chunked,
        "chunks kept other frames than one render"
    );
}

#[test]
fn a_webm_keeps_its_alpha() {
    if zgx_video::encode::ffmpeg().is_none() {
        eprintln!("no ffmpeg here: skipped");
        return;
    }
    let dir = app("webm");
    let mut clear = options(&dir, "out.webm");
    clear.transparent = true;
    render(&clear, |_| true).unwrap();
    let out = dir.join("out.webm");
    let said = probe(&out, "codec_name,color_primaries", "v").unwrap();
    assert_eq!(said, "vp9,bt709");
    // VP9's alpha rides as a side stream, which its own decoder reads back.
    let ffmpeg = zgx_video::encode::ffmpeg().unwrap();
    let decoded = std::process::Command::new(ffmpeg)
        .args(["-v", "error", "-c:v", "libvpx-vp9", "-i"])
        .arg(&out)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()
        .unwrap();
    let pixels = decoded.stdout;
    assert_eq!(pixels.len(), 64 * 48 * 4);
    assert!(pixel(&pixels, 64, 60, 40)[3] < 10, "nothing there is clear");
    assert!(pixel(&pixels, 64, 10, 10)[3] > 245, "the box is solid");
}

#[test]
fn an_av1_has_every_frame() {
    let Some(ffmpeg) = zgx_video::encode::ffmpeg() else {
        eprintln!("no ffmpeg here: skipped");
        return;
    };
    let encoders = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .output()
        .unwrap();
    if !String::from_utf8_lossy(&encoders.stdout).contains("av1") {
        eprintln!("no AV1 encoder here: skipped");
        return;
    }
    let dir = app("av1");
    let mut av1 = options(&dir, "out.mp4");
    av1.format = Some(Format::Av1);
    render(&av1, |_| true).unwrap();
    assert_eq!(
        probe(&dir.join("out.mp4"), "codec_name,nb_frames", "v").unwrap(),
        "av1,20"
    );
}

#[test]
fn a_chunked_webm_keeps_its_alpha_and_its_sound() {
    if zgx_video::encode::ffmpeg().is_none() {
        eprintln!("no ffmpeg here: skipped");
        return;
    }
    let dir = temp_dir("chunked-webm");
    let tone: Vec<f32> = (0..48000 * 2)
        .map(|i| (i as f32 * 0.03).sin() * 0.5)
        .collect();
    zgx_video::audio::write_wav(&dir.join("tone.wav"), &tone).unwrap();
    std::fs::write(
        dir.join("video.luau"),
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "W", w = 32, h = 32, fps = 30, duration = 30,
            render = function()
                return ui.div { w_full = true, h_full = true,
                    ui.div { w = 16, h = 16, bg = "#ff0000" },
                    video.audio { src = "tone.wav" } }
            end,
        } }
        "##,
    )
    .unwrap();
    let mut chunked = options(&dir, "out.webm");
    chunked.transparent = true;
    chunked.concurrency = 3;
    render(&chunked, |_| true).unwrap();
    let out = dir.join("out.webm");
    assert_eq!(probe(&out, "codec_name", "v").unwrap(), "vp9");
    assert_eq!(probe(&out, "codec_name,channels", "a").unwrap(), "opus,2");
}
