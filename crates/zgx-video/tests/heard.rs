//! A render's sound is what its own stage hears placed as it plays, with no
//! pass before it playing every frame again. Held here to what such a pass
//! (`placed_audio`: a stage of its own at a tenth of the size, one tick a
//! frame) places: the same clips in the same order, so the same mix, for
//! every way a render steps (motion blur's ticks between frames, another
//! rate, another size, frames skipped, chunks on threads of their own).
mod common;
use common::*;
use zgx_video::RenderOptions;
use zgx_video::render::{placed_audio, render_heard};

const PLACES: &str = r##"
local video = require("@video")
return video.compositions { video.composition {
    id = "H", w = 64, h = 36, fps = 30, duration = 90,
    render = function()
        local f = video.frame()
        return ui.div { w_full = true, h_full = true, bg = "#234",
            -- From the start, the whole way.
            video.audio { src = "a.wav", volume = 0.8 },
            -- A sequence's, faded in.
            video.sequence { from = 10, duration = 30, function()
                return video.audio { src = "b.wav", fade_in = 5, trim = 3 }
            end },
            -- Its loudness a function of its frame.
            video.audio { src = "a.wav", from = 5, duration = 40, volume = function(frame)
                return math.min(frame / 20, 1)
            end },
            -- Scenes in a series with a transition, each with its sound.
            video.sequence { from = 20, video.series {
                video.scene { duration = 20, function() return video.audio { src = "c.wav" } end },
                video.slide { from = "right", duration = 6 },
                video.scene { duration = 20, function() return video.audio { src = "b.wav", volume = 0.5 } end },
            } },
            -- A loop's, again at each turn.
            video.sequence { from = 50, duration = 30, video.loop { duration = 9, function()
                return video.audio { src = "c.wav", fade_out = 2 }
            end } },
            -- One frame long, on frame 9: a stage ticking at 24 fps of a 30 fps
            -- composition stands at 8.75 and 10, never in it.
            video.sequence { from = 9, duration = 1, function()
                return video.audio { src = "c.wav", volume = 0.25 }
            end },
            -- Placed on the last frame alone: a render skipping frames, or at a
            -- lower rate, still hears it.
            video.sequence { from = 89, function() return video.audio { src = "a.wav" } end },
            ui.div { w = 8, h = 8, bg = "#f80", x = f:map({ 0, 89 }, { 0, 56 }) },
        }
    end,
} }
"##;

#[test]
fn a_renders_own_stage_hears_what_a_pass_of_its_own_places() {
    if zgx_video::encode::ffmpeg().is_none() {
        eprintln!("no ffmpeg here: skipped");
        return;
    }
    let dir = temp_dir("heard");
    for name in ["a", "b", "c"] {
        let tone: Vec<f32> = (0..48000 * 2 * 4)
            .map(|i| (i as f32 * 0.01 * (name.as_bytes()[0] - b'`') as f32).sin() * 0.3)
            .collect();
        zgx_video::audio::write_wav(&dir.join(format!("{name}.wav")), &tone).unwrap();
    }
    std::fs::write(dir.join("video.luau"), PLACES).unwrap();
    let base = RenderOptions {
        script: dir.to_path_buf(),
        output: dir.join("out.mp4"),
        ..Default::default()
    };
    let (expected, fps) = placed_audio(&base).unwrap();
    assert!(expected.len() >= 5, "{} clips", expected.len());
    let cases: Vec<(&str, RenderOptions)> = vec![
        ("plain", base.clone()),
        (
            "motion blur",
            RenderOptions {
                samples: 4,
                ..base.clone()
            },
        ),
        (
            "at 24 fps",
            RenderOptions {
                fps: Some(24.),
                ..base.clone()
            },
        ),
        (
            "at 60 fps",
            RenderOptions {
                fps: Some(60.),
                ..base.clone()
            },
        ),
        (
            "half size",
            RenderOptions {
                scale: 0.5,
                ..base.clone()
            },
        ),
        (
            "every 4th frame",
            RenderOptions {
                every_nth: 4,
                ..base.clone()
            },
        ),
        (
            "in 3 chunks",
            RenderOptions {
                concurrency: 3,
                ..base.clone()
            },
        ),
        (
            "in 3 chunks, blurred, at 24 fps",
            RenderOptions {
                concurrency: 3,
                samples: 2,
                fps: Some(24.),
                ..base.clone()
            },
        ),
    ];
    for (name, options) in cases {
        let (report, heard) = render_heard(&options, |_| true).unwrap();
        let (clips, heard_fps) = heard.unwrap_or_else(|| panic!("{name}: heard nothing"));
        assert_eq!(heard_fps, fps, "{name}");
        assert_eq!(clips, expected, "{name}: its stage heard other sounds");
        assert!(report.frames > 0);
    }
}
