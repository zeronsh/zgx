//! The Luau side: compositions, frames, sequences, interpolation and springs.
mod common;
use common::*;
use serde_json::json;
use zgx_video::compositions;

fn app(name: &str, code: &str) -> std::path::PathBuf {
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
