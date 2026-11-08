//! The Luau side: compositions, frames, sequences, interpolation and springs.
mod common;
use common::*;
use serde_json::json;
use zgx_video::compositions;

fn app(name: &str, code: &str) -> TempDir {
    let dir = temp_dir(name);
    std::fs::write(dir.join("video.luau"), code).unwrap();
    dir
}

/// A script's result, as a bar's width: `value` drawn as that many pixels wide.
fn bar(body: &str) -> String {
    format!(
        r##"
        local video = require("@video")
        return video.compositions {{ video.composition {{
            id = "B", w = 300, h = 20, fps = 30, duration = 100,
            render = function(props)
                local f = video.frame()
                {body}
                return ui.div {{ w = 300, h = 20, ui.div {{ h = 20, w = value, bg = "#ff0000" }} }}
            end,
        }} }}
        "##
    )
}
fn width_at(dir: &std::path::Path, frame: u64) -> u32 {
    let mut stage = script_stage(dir, json!({}));
    stage.step_to(frame).unwrap();
    let mut pixels = Vec::new();
    stage.stage.draw(&mut pixels).unwrap();
    (0..300u32)
        .rev()
        .find(|x| pixel(&pixels, 300, *x, 10)[0] > 128)
        .map_or(0, |x| x + 1)
}

#[test]
fn a_script_lists_its_compositions_and_their_props_decide_their_measure() {
    let dir = app(
        "list",
        r##"
        local video = require("@video")
        local draw = function() return ui.div {} end
        return video.compositions {
            video.composition { id = "A", w = 640, h = 360, fps = 24, duration = 48, render = draw },
            video.composition {
                id = "B", fps = 60, props = { seconds = 2, title = "x" },
                duration = function(props) return props.seconds * 60 end,
                w = function(props) return props.seconds * 100 end,
                render = draw,
            },
        }
        "##,
    );
    let list = compositions(&dir, Some("B"), &json!({ "seconds": 3 })).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(
        (
            list[0].id.as_str(),
            list[0].width,
            list[0].fps,
            list[0].duration
        ),
        ("A", 640., 24., 48)
    );
    assert_eq!(list[1].duration, 180);
    assert_eq!(list[1].width, 300.);
    assert_eq!(list[1].props, json!({ "seconds": 3, "title": "x" }));
    assert_eq!(list[1].defaults, json!({ "seconds": 2, "title": "x" }));
}

#[test]
fn a_script_with_no_compositions_says_so() {
    let dir = app("none", "return ui.div {}");
    let error = compositions(&dir, None, &json!({})).unwrap_err();
    assert!(error.contains("no compositions"), "{error}");
}

#[test]
fn interpolate_holds_at_its_ends_and_extends_when_told() {
    let dir = app(
        "map",
        &bar("local value = f:map({ 10, 20, 40 }, { 0, 100, 200 })"),
    );
    assert_eq!(width_at(&dir, 5), 0);
    assert_eq!(width_at(&dir, 15), 50);
    assert_eq!(width_at(&dir, 30), 150);
    assert_eq!(width_at(&dir, 90), 200);
    let dir = app(
        "extend",
        &bar("local value = f:map({ 0, 10 }, { 0, 10 }, { clamp = false })"),
    );
    assert_eq!(width_at(&dir, 50), 50);
    let dir = app(
        "eased",
        &bar("local value = f:map({ 0, 10 }, { 0, 100 }, { ease = 'inQuad' })"),
    );
    assert_eq!(width_at(&dir, 5), 25);
}

#[test]
fn a_spring_is_a_function_of_the_frame_alone() {
    // Critically damped, in closed form: 1 - e^(-wt)(1 + wt), w = 10, t = 1/3 s.
    let dir = app(
        "spring",
        &bar("local value = f:spring { from = 0, to = 200, stiffness = 100, damping = 20 }"),
    );
    let t = 10. / 30.;
    let want = 200. * (1. - f64::exp(-10. * t) * (1. + 10. * t));
    assert!((f64::from(width_at(&dir, 10)) - want).abs() <= 1., "{want}");
    assert!(width_at(&dir, 99) >= 199);
    // Bouncy, it passes its end; told not to, it holds there.
    let over = app(
        "over",
        &bar("local value = f:spring { to = 200, stiffness = 200, damping = 6 }"),
    );
    assert!(width_at(&over, 7) > 200);
    let held = app(
        "held",
        &bar(
            "local value = f:spring { to = 200, stiffness = 200, damping = 6, overshoot = false }",
        ),
    );
    assert_eq!(width_at(&held, 7), 200);
}

#[test]
fn a_sequence_shows_only_in_its_frames_and_its_time_starts_with_it() {
    let dir = app(
        "sequence",
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "S", w = 300, h = 20, fps = 30, duration = 100,
            render = function()
                return ui.overlay { w = 300, h = 20,
                    video.sequence { from = 10, duration = 20, name = "Bar", function(t)
                        return ui.div { h = 20, w = function() return 5 + t:get() end, bg = "#ff0000" }
                    end },
                }
            end,
        } }
        "##,
    );
    assert_eq!(width_at(&dir, 9), 0);
    assert_eq!(width_at(&dir, 10), 5);
    assert_eq!(width_at(&dir, 29), 24);
    assert_eq!(width_at(&dir, 30), 0);
    let stage = script_stage(&dir, json!({}));
    let tracks = stage.tracks();
    assert_eq!(tracks.len(), 1);
    assert_eq!((tracks[0].name.as_str(), tracks[0].from), ("Bar", 10));
    assert_eq!(tracks[0].duration, Some(20));
}

#[test]
fn nested_sequences_add_their_offsets_and_components_read_the_time_they_mount_in() {
    let dir = app(
        "nested",
        r##"
        local video = require("@video")
        local Bar = ui.component(function()
            local t = video.frame()
            return ui.div { h = 20, w = function() return 1 + t:get() end, bg = "#ff0000" }
        end)
        return video.compositions { video.composition {
            id = "N", w = 300, h = 20, fps = 30, duration = 100,
            render = function()
                return ui.overlay { w = 300, h = 20,
                    video.sequence { from = 10, function()
                        return video.sequence { from = 5, Bar {} }
                    end },
                }
            end,
        } }
        "##,
    );
    assert_eq!(width_at(&dir, 14), 0);
    assert_eq!(width_at(&dir, 15), 1);
    assert_eq!(width_at(&dir, 25), 11);
}

#[test]
fn sounds_given_to_a_sequence_as_tables_start_where_it_places_them() {
    // Children given as tables are built before the sequence is: a series, a
    // loop, a sound or a sequence among them takes its time where it mounts.
    let dir = app(
        "nested-sounds",
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "S", w = 32, h = 32, fps = 30, duration = 120,
            render = function()
                return ui.div { w_full = true, h_full = true,
                    video.sequence { from = 20, video.series {
                        video.scene { duration = 20, function() return video.audio { src = "one.wav" } end },
                        video.slide { from = "right", duration = 6 },
                        video.scene { duration = 20, function() return video.audio { src = "two.wav" } end },
                    } },
                    video.sequence { from = 50, duration = 20, video.loop { duration = 10, function()
                        return video.audio { src = "turn.wav" }
                    end } },
                    video.sequence { from = 80, video.audio { src = "direct.wav" } },
                    video.sequence { from = 90, video.sequence { from = 5, function()
                        return video.audio { src = "inner.wav" }
                    end } },
                }
            end,
        } }
        "##,
    );
    let mut stage = script_stage(&dir, json!({}));
    stage.step_to(119).unwrap();
    let placed: Vec<(String, u64)> = stage
        .audio()
        .iter()
        .map(|clip| {
            let name = clip.src.file_name().unwrap().to_string_lossy();
            (name.into_owned(), clip.from)
        })
        .collect();
    let expected: Vec<(String, u64)> = [
        ("one.wav", 20),
        ("two.wav", 34),
        ("turn.wav", 50),
        ("turn.wav", 60),
        ("direct.wav", 80),
        ("inner.wav", 95),
    ]
    .into_iter()
    .map(|(name, from)| (name.to_owned(), from))
    .collect();
    assert_eq!(placed, expected);
}

#[test]
fn a_sound_in_a_loop_plays_on_every_pass() {
    let dir = app(
        "loop-click",
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "L", w = 32, h = 32, fps = 30, duration = 60,
            render = function()
                return ui.div { w_full = true, h_full = true,
                    video.sequence { from = 10, video.loop { duration = 12, times = 3, function()
                        return video.audio { src = "click.wav" }
                    end } },
                }
            end,
        } }
        "##,
    );
    // A click: one sample at full scale, the rest silence.
    let mut click = vec![0f32; 2 * 480];
    click[0] = 1.;
    click[1] = 1.;
    zgx_video::audio::write_wav(&dir.join("click.wav"), &click).unwrap();
    let mut stage = script_stage(&dir, json!({}));
    stage.step_to(59).unwrap();
    let clips = stage.audio();
    // One stage, and one virtual clock, a thread at a time.
    drop(stage);
    let starts: Vec<u64> = clips.iter().map(|clip| clip.from).collect();
    assert_eq!(starts, [10, 22, 34], "one a pass, at its start");
    // Ticking four times a frame (motion blur), its passes start on the
    // same frames.
    let mut blurred = zgx_video::ScriptStage::new(
        &dir,
        None,
        &json!({}),
        1.,
        zgx::scene::Color(0, 0, 0, 255),
        zgx_video::Timing {
            samples: 4,
            ..Default::default()
        },
    )
    .unwrap();
    blurred.film.step_to(59).unwrap();
    let blurred: Vec<u64> = blurred.audio().iter().map(|clip| clip.from).collect();
    assert_eq!(blurred, [10, 22, 34], "with motion blur");
    // Mixed, the click sounds at each pass's start: 48,000 samples a second.
    let mix = zgx_video::audio::mixed_file(&clips, 30., 60)
        .unwrap()
        .unwrap();
    let sound = zgx_video::audio::decode(&mix).unwrap();
    let _ = std::fs::remove_file(&mix);
    let heard: Vec<usize> = (0..sound.0.len() / 2)
        .filter(|frame| sound.0[frame * 2] > 0.5)
        .collect();
    assert_eq!(heard, [16_000, 35_200, 54_400]);
}

#[test]
fn what_starts_as_a_sequence_mounts_starts_then() {
    let dir = app(
        "mount",
        r##"
        local video = require("@video")
        local Bar = ui.component(function()
            local box = ui.ref()
            ui.on_mount(function()
                anime.animate(box:get(), { x = 100, duration = 300, ease = "linear" })
            end)
            return ui.div { ref = box, h = 20, w = 20, bg = "#ff0000" }
        end)
        return video.compositions { video.composition {
            id = "M", w = 300, h = 20, fps = 30, duration = 200,
            render = function()
                return ui.overlay { w = 300, h = 20, video.sequence { from = 60, Bar {} } }
            end,
        } }
        "##,
    );
    // Two seconds in it has only just begun, and 300 ms on, it's done.
    assert!(width_at(&dir, 62) <= 50);
    assert_eq!(width_at(&dir, 72), 120);
}

#[test]
fn a_series_overlaps_its_scenes_by_their_transition() {
    let dir = app(
        "series",
        r##"
        local video = require("@video")
        local function scene(duration, color)
            return video.scene { duration = duration, name = color, function(t)
                return ui.div { w_full = true, h_full = true, bg = color }
            end }
        end
        local list = {
            scene(30, "#ff0000"),
            video.fade { duration = 10, ease = "linear" },
            scene(30, "#0000ff"),
        }
        return video.compositions { video.composition {
            id = "T", w = 20, h = 20, fps = 30, duration = video.series_length(list),
            render = function() return video.series(list) end,
        } }
        "##,
    );
    let mut stage = script_stage(&dir, json!({}));
    assert_eq!(stage.composition().duration, 50);
    let mut pixels = Vec::new();
    let mut at = |frame| {
        stage.step_to(frame).unwrap();
        stage.stage.draw(&mut pixels).unwrap();
        pixel(&pixels, 20, 10, 10)
    };
    assert_eq!(at(10)[..3], [255, 0, 0]);
    let mid = at(24);
    assert!(mid[0] > 80 && mid[2] > 80, "both show: {mid:?}");
    assert_eq!(at(40)[..3], [0, 0, 255]);
}

#[test]
fn a_shader_transition_sees_the_scene_before_it() {
    let dir = app(
        "wipe",
        r##"
        local video = require("@video")
        local function scene(duration, color)
            return video.scene { duration = duration, function()
                return ui.div { w_full = true, h_full = true, bg = color }
            end }
        end
        local list = { scene(30, "#ff0000"), video.wipe { duration = 10, ease = "linear", soft = 0.01 }, scene(30, "#0000ff") }
        return video.compositions { video.composition {
            id = "W", w = 100, h = 20, fps = 30, duration = 50,
            render = function() return video.series(list) end,
        } }
        "##,
    );
    let mut stage = script_stage(&dir, json!({}));
    stage.step_to(25).unwrap();
    let mut pixels = Vec::new();
    stage.stage.draw(&mut pixels).unwrap();
    let (left, right) = (pixel(&pixels, 100, 5, 10), pixel(&pixels, 100, 95, 10));
    assert!(
        left[2] > 200 && left[0] < 40,
        "the new scene has come: {left:?}"
    );
    assert!(
        right[0] > 200 && right[2] < 40,
        "the old is still there: {right:?}"
    );
}

#[test]
fn input_props_reach_the_render() {
    let dir = app("props", &bar("local value = props.width or 3"));
    let mut stage = script_stage(&dir, json!({ "width": 44 }));
    let mut pixels = Vec::new();
    stage.stage.draw(&mut pixels).unwrap();
    assert_eq!(pixel(&pixels, 300, 43, 10)[0], 255);
    assert_eq!(pixel(&pixels, 300, 45, 10)[0], 0);
    drop(stage);
    assert_eq!(width_at(&dir, 0), 3);
}

#[test]
fn a_script_error_while_it_plays_stops_the_render_and_says_where() {
    let dir = app(
        "broken",
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "E", w = 20, h = 20, duration = 10,
            render = function()
                local f = video.frame()
                return ui.div { w = function() if f:get() > 2 then error("boom") end return 3 end }
            end,
        } }
        "##,
    );
    let mut stage = script_stage(&dir, json!({}));
    let error = stage.step_to(5).unwrap_err();
    assert!(error.contains("boom"), "{error}");
}

#[test]
fn audio_data_draws_a_tones_band_at_its_time() {
    let dir = temp_dir("audio-data");
    // Silence for half a second, then a 1 kHz tone for one.
    let rate = 48_000usize;
    let tone: Vec<f32> = (0..rate * 3 / 2)
        .flat_map(|i| {
            let v = if i < rate / 2 {
                0.
            } else {
                (2. * std::f32::consts::PI * 1000. * i as f32 / rate as f32).sin() * 0.9
            };
            [v, v]
        })
        .collect();
    zgx_video::audio::write_wav(&dir.join("tone.wav"), &tone).unwrap();
    std::fs::write(
        dir.join("video.luau"),
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "V", w = 32, h = 100, fps = 30, duration = 45,
            render = function()
                local song = video.audio_data("tone.wav")
                assert(math.abs(song.seconds - 1.5) < 0.01, "its length")
                local row = { direction = "row", items = "end", w = 32, h = 100, bg = "#000000" }
                for i = 1, 32 do
                    table.insert(row, ui.div { w = 1, bg = "#ff0000",
                        h = function() return song:bars({ count = 32 })[i] * 100 end })
                end
                return ui.div(row)
            end,
        } }
        "##,
    )
    .unwrap();
    let mut stage = script_stage(&dir, json!({}));
    let mut pixels = Vec::new();
    let height = |pixels: &[u8], x: u32| {
        (0..100u32)
            .filter(|y| pixel(pixels, 32, x, *y)[0] > 128)
            .count()
    };
    let band = ((1000f32 / 40.).ln() / (16_000f32 / 40.).ln() * 32.) as u32;
    stage.step_to(6).unwrap();
    stage.draw(&mut pixels).unwrap();
    assert_eq!(height(&pixels, band), 0, "silent at 0.2 s");
    stage.step_to(30).unwrap();
    stage.draw(&mut pixels).unwrap();
    assert!(
        height(&pixels, band) > 90,
        "its band at 1 s: {}",
        height(&pixels, band)
    );
    assert!(
        height(&pixels, 3) < 30,
        "a far band: {}",
        height(&pixels, 3)
    );
}

#[test]
fn captions_show_their_cue_at_its_time_word_by_word() {
    let dir = temp_dir("captions");
    std::fs::write(
        dir.join("subs.srt"),
        "1\n00:00:01,000 --> 00:00:02,000\nMMMM WWWW\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("video.luau"),
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "C", w = 300, h = 60, fps = 30, duration = 90,
            render = function()
                local cues = video.captions_of("subs.srt")
                assert(#cues == 1 and cues[1].text == "MMMM WWWW", "its cue")
                return ui.column { w = 300, h = 60, bg = "#000000", items = "center",
                    video.captions { cues = cues, words = true, text_size = 40,
                        color = "#0000ff", active_color = "#ffff00", upcoming_color = "#00ff00" } }
            end,
        } }
        "##,
    )
    .unwrap();
    let mut stage = script_stage(&dir, json!({}));
    let mut pixels = Vec::new();
    // Pixels near a colour, left of the middle and right of it.
    let count = |pixels: &[u8], rgb: [u8; 3]| {
        let mut sides = (0, 0);
        for y in 0..60u32 {
            for x in 0..300u32 {
                let p = pixel(pixels, 300, x, y);
                if (0..3).all(|c| p[c].abs_diff(rgb[c]) < 60) {
                    if x < 150 { sides.0 += 1 } else { sides.1 += 1 }
                }
            }
        }
        sides
    };
    stage.step_to(15).unwrap();
    stage.draw(&mut pixels).unwrap();
    assert_eq!(
        count(&pixels, [255, 255, 0]),
        (0, 0),
        "nothing before its cue"
    );
    stage.step_to(36).unwrap();
    stage.draw(&mut pixels).unwrap();
    let (yellow, green) = (count(&pixels, [255, 255, 0]), count(&pixels, [0, 255, 0]));
    assert!(
        yellow.0 > 50 && yellow.1 == 0,
        "the first word is being said: {yellow:?}"
    );
    assert!(green.1 > 50, "the second is to come: {green:?}");
    stage.step_to(54).unwrap();
    stage.draw(&mut pixels).unwrap();
    let (yellow, blue) = (count(&pixels, [255, 255, 0]), count(&pixels, [0, 0, 255]));
    assert!(
        yellow.1 > 50 && yellow.0 == 0,
        "then the second: {yellow:?}"
    );
    assert!(blue.0 > 50, "the first said: {blue:?}");
    stage.step_to(70).unwrap();
    stage.draw(&mut pixels).unwrap();
    assert_eq!(count(&pixels, [0, 0, 255]), (0, 0), "gone after it");
}

#[test]
fn a_loop_starts_its_time_again_each_pass_and_mounts_again() {
    let dir = app(
        "loop",
        r##"
        local video = require("@video")
        local Grow = ui.component(function()
            local box = ui.ref()
            ui.on_mount(function()
                anime.animate(box:get(), { w = { 1, 101 }, duration = 300, ease = "linear" })
            end)
            return ui.div { ref = box, h = 10, w = 1, bg = "#00ff00" }
        end)
        return video.compositions { video.composition {
            id = "L", w = 300, h = 30, fps = 30, duration = 100,
            render = function()
                return ui.overlay { w = 300, h = 30,
                    video.loop { duration = 20, times = 3, function(t)
                        return ui.column {
                            ui.div { h = 10, w = function() return 1 + t:get() * 5 end, bg = "#ff0000" },
                            Grow {},
                            -- A sequence inside it follows the pass's time.
                            video.sequence { from = 10, layout = "none",
                                ui.div { h = 1, w = 300, bg = "#0000ff" } },
                        }
                    end },
                }
            end,
        } }
        "##,
    );
    let mut stage = script_stage(&dir, json!({}));
    let mut pixels = Vec::new();
    let mut at = |frame: u64| {
        stage.step_to(frame).unwrap();
        stage.draw(&mut pixels).unwrap();
        let red = (0..300u32)
            .filter(|x| pixel(&pixels, 300, *x, 5)[0] > 128)
            .count();
        let green = (0..300u32)
            .filter(|x| pixel(&pixels, 300, *x, 15)[1] > 128)
            .count();
        let blue = (0..300u32).any(|x| pixel(&pixels, 300, x, 20)[2] > 128);
        (red, green, blue)
    };
    assert_eq!(at(5).0, 26);
    assert!(!at(5).2, "its sequence not yet");
    let (red, green, blue) = at(25);
    assert_eq!(red, 26, "its time from 0 again in the second pass");
    // Five frames of its 300 ms: about half grown, where it would be whole
    // had it not started again.
    assert!(
        (30..70).contains(&green),
        "and what started as it mounted, started again: {green}"
    );
    assert!(!blue);
    assert!(at(32).2, "its sequence, ten frames into the pass");
    assert_eq!(at(65).0, 0, "three passes and done");
}

#[test]
fn a_freeze_holds_its_frame() {
    let dir = app(
        "freeze2",
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "F", w = 300, h = 20, fps = 30, duration = 100,
            render = function()
                return ui.overlay { w = 300, h = 20,
                    video.freeze { frame = 12, function(t)
                        return ui.div { h = 20, w = t:map({ 0, 100 }, { 0, 300 }), bg = "#ff0000" }
                    end } }
            end,
        } }
        "##,
    );
    assert_eq!(width_at(&dir, 0), 36);
    assert_eq!(width_at(&dir, 80), 36);
}

#[test]
fn captions_in_a_loop_come_again_each_pass() {
    let dir = temp_dir("loop-captions");
    std::fs::write(
        dir.join("subs.srt"),
        "1\n00:00:00,000 --> 00:00:00,500\nMMMM\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("video.luau"),
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "L", w = 200, h = 40, fps = 30, duration = 90,
            render = function()
                return ui.column { w = 200, h = 40, bg = "#000000",
                    video.loop { duration = 30, layout = "none",
                        video.captions { src = "subs.srt", text_size = 30, color = "#ff0000" } } }
            end,
        } }
        "##,
    )
    .unwrap();
    let mut stage = script_stage(&dir, json!({}));
    let mut pixels = Vec::new();
    let mut red = |frame: u64| {
        stage.step_to(frame).unwrap();
        stage.draw(&mut pixels).unwrap();
        (0..40u32)
            .flat_map(|y| (0..200u32).map(move |x| (x, y)))
            .filter(|(x, y)| pixel(&pixels, 200, *x, *y)[0] > 128)
            .count()
    };
    assert!(red(5) > 30, "its cue in the first pass");
    assert_eq!(red(20), 0, "gone after half a second");
    assert!(red(35) > 30, "and back in the second pass");
}
