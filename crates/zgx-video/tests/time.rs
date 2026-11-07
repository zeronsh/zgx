//! Everything that moves takes its time from the frame number: a frame is the
//! same however fast it's made, and shows what its time says.
mod common;
use common::*;
use serde_json::json;
use std::time::Duration;

/// Its frames' hashes, each `pause` of wall clock apart.
fn film(script: &str, frames: u64, pause: Duration) -> Vec<u64> {
    let mut stage = inline(script, 200., 100., 30.);
    let mut pixels = Vec::new();
    let mut hashes = Vec::new();
    for _ in 0..frames {
        stage.draw(&mut pixels).unwrap();
        hashes.push(hash(&pixels));
        std::thread::sleep(pause);
        stage.step().unwrap();
    }
    hashes
}
const EVERYTHING: &str = r##"
local open = ui.state("open", false)
local ticks = ui.state("ticks", 0)
ui.after(100, function() open:set(true) end)
ui.every(50, function() ticks:add(1) end)
local box = ui.ref()
ui.on_mount(function()
    anime.animate(box:get(), { x = 100, duration = 400, ease = "outCubic" })
end)
local wide = ui.computed(function() return if open:get() then 150 else 10 end)
return ui.column {
    w = 200, h = 100, bg = "#123",
    ui.div { ref = box, w = 20, h = 10, bg = "#f80" },
    ui.div { h = 10, w = ui.tween(wide, { duration = 200 }), bg = "#0f8" },
    ui.div { h = 10, w = ui.spring(wide), bg = "#08f" },
    ui.div { h = 10, w = function() return ticks:get() * 4 end, bg = "#f08" },
    ui.div { h = 10, w = function() return math.random() * 100 end, bg = "#ff0" },
    ui.div { h = 10, w = function() ticks:get(); return ui.now() % 100 end, bg = "#0ff" },
    ui.div { h = 10, w = function() ticks:get(); return os.clock() * 50 end, bg = "#f0f" },
    ui.div {
        h = 20, w = 200,
        bg_shader = ui.shader("fn fill(input: FillInput) -> vec4<f32> { return vec4<f32>(fract(input.time), 0., 0., 1.); }"),
        shader_clock = true,
    },
}
"##;

#[test]
fn the_wall_clock_changes_no_frame() {
    let quick = film(EVERYTHING, 20, Duration::ZERO);
    let slow = film(EVERYTHING, 20, Duration::from_millis(15));
    assert_eq!(quick, slow);
    let distinct: std::collections::HashSet<_> = quick.iter().collect();
    assert!(distinct.len() > 15, "it moves: {} distinct", distinct.len());
}

/// How far a bar `bar` reaches, at the row `y`.
fn reach(pixels: &[u8], width: u32, y: u32, rgb: [u8; 3]) -> u32 {
    (0..width)
        .rev()
        .find(|x| pixel(pixels, width, *x, y)[..3] == rgb)
        .map_or(0, |x| x + 1)
}

#[test]
fn an_animation_ends_at_its_time_however_fast_it_was_made() {
    let mut stage = inline(
        r##"
        local box = ui.ref()
        ui.on_mount(function()
            anime.animate(box:get(), { x = 100, duration = 400, ease = "linear" })
        end)
        return ui.div { w = 200, h = 100, bg = "#000", ui.div { ref = box, w = 20, h = 10, bg = "#ff0000" } }
        "##,
        200.,
        100.,
        30.,
    );
    // 200 ms in, at 30 fps: about half way, however little wall clock passed.
    let mut pixels = Vec::new();
    stage.step_to(6).unwrap();
    stage.draw(&mut pixels).unwrap();
    let halfway = reach(&pixels, 200, 5, [255, 0, 0]);
    assert!(
        (60..=75).contains(&halfway),
        "at 200 ms it reaches {halfway}"
    );
    stage.step_to(14).unwrap();
    stage.draw(&mut pixels).unwrap();
    assert_eq!(
        reach(&pixels, 200, 5, [255, 0, 0]),
        120,
        "and by 466 ms it's there"
    );
}

#[test]
fn timers_fire_at_their_own_time_between_frames() {
    let mut stage = inline(
        r##"
        local log = ui.state("log", 0)
        ui.after(50, function() log:add(1) end)
        ui.every(40, function(): boolean? log:add(10); return log:peek() < 30 end)
        return ui.div { w = 200, h = 10, bg = "#000",
            ui.div { h = 10, w = function() return log:get() end, bg = "#ff0000" } }
        "##,
        200.,
        10.,
        30.,
    );
    let mut pixels = Vec::new();
    let mut at = |stage: &mut zgx_video::Stage, frame| {
        stage.step_to(frame).unwrap();
        stage.draw(&mut pixels).unwrap();
        reach(&pixels, 200, 5, [255, 0, 0])
    };
    assert_eq!(at(&mut stage, 1), 0, "33 ms");
    assert_eq!(at(&mut stage, 2), 11, "66 ms: 40 and 50 have passed");
    assert_eq!(at(&mut stage, 3), 21, "100 ms: 80 has");
    assert_eq!(at(&mut stage, 10), 31, "120 ms, and it stopped");
}

#[test]
fn shader_time_is_the_frames() {
    let mut stage = inline(
        r##"
        return ui.div {
            w = 20, h = 20,
            bg_shader = ui.shader("fn fill(input: FillInput) -> vec4<f32> { return vec4<f32>(input.time / 2., 0., 0., 1.); }"),
            shader_clock = true,
        }
        "##,
        20.,
        20.,
        30.,
    );
    let mut pixels = Vec::new();
    stage.draw(&mut pixels).unwrap();
    assert!(pixel(&pixels, 20, 10, 10)[0] <= 1);
    stage.step_to(30).unwrap();
    stage.draw(&mut pixels).unwrap();
    // One second of two: half, in sRGB's bytes as written.
    let red = pixel(&pixels, 20, 10, 10)[0];
    assert!((120..=135).contains(&red), "after a second it reads {red}");
}

#[test]
fn a_gif_plays_at_the_frames_time() {
    let dir = temp_dir("gif");
    write_gif(&dir.join("two.gif"));
    std::fs::write(
        dir.join("video.luau"),
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "G", w = 20, h = 20, fps = 30, duration = 30,
            render = function() return ui.image { src = "two.gif", w = 20, h = 20 } end,
        } }
        "##,
    )
    .unwrap();
    let mut stage = script_stage(&dir, json!({}));
    let mut pixels = Vec::new();
    let red = |p: &[u8]| pixel(p, 20, 10, 10)[0] > 200;
    let mut seen = Vec::new();
    for frame in [0, 2, 3, 5, 6, 8] {
        stage.step_to(frame).unwrap();
        stage.draw(&mut pixels).unwrap();
        seen.push(red(&pixels));
    }
    // A hundred ms each: three frames red, three blue, and on.
    assert_eq!(seen, [true, true, false, false, true, true]);
}

#[test]
fn a_picture_is_there_on_the_first_frame() {
    let dir = temp_dir("png");
    write_png(&dir.join("red.png"));
    std::fs::write(
        dir.join("video.luau"),
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "P", w = 20, h = 20, fps = 30, duration = 3,
            render = function() return ui.image { src = "red.png", w = 20, h = 20 } end,
        } }
        "##,
    )
    .unwrap();
    let mut stage = script_stage(&dir, json!({}));
    let mut pixels = Vec::new();
    stage.draw(&mut pixels).unwrap();
    assert_eq!(pixel(&pixels, 20, 10, 10)[..3], [255, 0, 0]);
}

#[test]
fn a_large_picture_is_there_on_the_first_frame() {
    // Large enough that its pixels decode off the thread at the size it's
    // drawn, as a window's would, rather than with the file.
    let dir = temp_dir("large-png");
    let (width, height) = (2400, 800);
    let file = std::fs::File::create(dir.join("wide.png")).unwrap();
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&[0, 0, 255, 255].repeat((width * height) as usize))
        .unwrap();
    drop(writer);
    std::fs::write(
        dir.join("video.luau"),
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "P", w = 60, h = 20, fps = 30, duration = 3,
            render = function() return ui.image { src = "wide.png", w = 60, h = 20 } end,
        } }
        "##,
    )
    .unwrap();
    // As a render or still makes it: what the script waits on, waited for.
    let options = zgx_video::RenderOptions {
        script: dir.to_path_buf(),
        output: dir.join("unused"),
        ..Default::default()
    };
    let still = zgx_video::render_still(&options, 0, None).unwrap();
    assert_eq!(pixel(&still.pixels, 60, 30, 10)[..3], [0, 0, 255]);
}

#[test]
fn a_composition_names_the_fonts_beside_it() {
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../zgx-gpu/tests/fonts/GeistVariable-subset.ttf");
    let script = |family: &str| {
        format!(
            r##"
            local video = require("@video")
            return video.compositions {{ video.composition {{
                id = "F", w = 200, h = 60, fps = 30, duration = 1,
                render = function()
                    return ui.div {{ w = 200, h = 60, bg = "#000",
                        ui.text {{ "Hello", font = "{family}", text_size = 40, color = "#fff" }} }}
                end,
            }} }}
            "##
        )
    };
    let drawn = |dir: &std::path::Path, family: &str| {
        std::fs::write(dir.join("video.luau"), script(family)).unwrap();
        let mut pixels = Vec::new();
        script_stage(dir, json!({})).draw(&mut pixels).unwrap();
        hash(&pixels)
    };
    let dir = temp_dir("fonts");
    // Without the font, its family falls back to the system's, as an unknown
    // one does.
    let fallback = drawn(&dir, "No Such Family");
    assert_eq!(drawn(&dir, "Geist"), fallback);
    std::fs::create_dir_all(dir.join("fonts")).unwrap();
    std::fs::copy(&font, dir.join("fonts/Geist.ttf")).unwrap();
    std::fs::write(dir.join("fonts/notes.txt"), "not a font").unwrap();
    // Named as a font but not one: said, and left out.
    std::fs::write(dir.join("fonts/broken.ttf"), "not a font either").unwrap();
    let fonts = zgx_video::script::fonts_beside(&dir.join("video.luau"));
    assert_eq!(fonts.len(), 1);
    assert!(fonts[0].families().iter().any(|family| family == "Geist"));
    assert_ne!(drawn(&dir, "Geist"), fallback);
    assert_eq!(drawn(&dir, "No Such Family"), fallback);
}

#[test]
fn a_looping_animation_keeps_the_frames_time() {
    // A loop a window's renderer might play by itself, off the wall clock.
    let script = r##"
        local box = ui.ref()
        ui.on_mount(function()
            anime.animate(box:get(), { opacity = { 0, 1 }, duration = 400, loop = true, alternate = true, ease = "linear" })
        end)
        return ui.div { w = 40, h = 40, bg = "#000",
            ui.div { ref = box, w = 40, h = 40, rounded = 8, bg = "#ff0000" } }
    "##;
    let quick = film(script, 60, Duration::ZERO);
    let slow = film(script, 60, Duration::from_millis(5));
    assert_eq!(quick, slow);
    let distinct: std::collections::HashSet<_> = quick.iter().collect();
    assert!(distinct.len() > 10, "{} distinct", distinct.len());
}

#[test]
fn the_date_and_chance_are_the_same_each_time() {
    let script = r##"
        local stamp = ui.now()
        local chance = math.random()
        local now = os.time()
        return ui.column { w = 60, h = 30,
            ui.div { h = 10, w = (stamp % 1000) / 1000 * 60 + 1, bg = "#ff0000" },
            ui.div { h = 10, w = chance * 60 + 1, bg = "#00ff00" },
            ui.div { h = 10, w = (now % 60) + 1, bg = "#0000ff" },
        }
    "##;
    let once = film(script, 1, Duration::ZERO);
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(once, film(script, 1, Duration::ZERO));
}

/// How many pixels of `a` and `b` differ by more than a little, and where the
/// first is.
fn differ(a: &[u8], b: &[u8], width: u32) -> (usize, (u32, u32)) {
    let mut count = 0;
    let mut first = (0, 0);
    for (i, (x, y)) in a.chunks(4).zip(b.chunks(4)).enumerate() {
        if x.iter().zip(y).any(|(p, q)| p.abs_diff(*q) > 6) {
            if count == 0 {
                first = (i as u32 % width, i as u32 / width);
            }
            count += 1;
        }
    }
    (count, first)
}

#[test]
fn a_frame_is_the_same_whether_those_before_it_were_drawn_or_not() {
    // A blurred circle in a box fading and scaling in, then out of it.
    let script = r##"
        local n = ui.state("n", 0)
        ui.every(33, function() n:add(1) end)
        return ui.div { w = 500, h = 300, bg = "#120a2c",
            ui.div {
                absolute = true, top = 0, left = 0, right = 0, bottom = 0,
                opacity = function() return math.min(n:get() / 10, 1) end,
                scale = function() return 1.25 - 0.25 * math.min(n:get() / 10, 1) end,
                ui.div {
                    absolute = true, left = 40, top = 30, w = 200, h = 200, rounded = 100, blur = 24,
                    gradient = { angle = 135, stops = { "#ff5a5f", "#ff5a5f55" } },
                    x = function() return math.sin(n:get() * 0.2) * 60 end,
                },
            },
        }
    "##;
    let at = 20;
    let drawn = std::thread::spawn(move || {
        let mut stage = inline(script, 500., 300., 30.);
        let mut pixels = Vec::new();
        for _ in 0..at {
            stage.draw(&mut pixels).unwrap();
            stage.step().unwrap();
        }
        stage.draw(&mut pixels).unwrap();
        pixels
    });
    let fresh = std::thread::spawn(move || {
        let mut stage = inline(script, 500., 300., 30.);
        let mut pixels = Vec::new();
        stage.step_to(at).unwrap();
        stage.draw(&mut pixels).unwrap();
        pixels
    });
    let (drawn, fresh) = (drawn.join().unwrap(), fresh.join().unwrap());
    let (count, first) = differ(&drawn, &fresh, 500);
    assert!(count == 0, "{count} pixels differ, first at {first:?}");
}

#[test]
fn repainting_what_changed_draws_what_painting_it_all_draws() {
    let script = r##"
        local n = ui.state("n", 0)
        ui.every(33, function() n:add(1) end)
        local orb = function(left, size, color, speed)
            return ui.div {
                absolute = true, left = left, top = 30, w = size, h = size, rounded = size / 2, blur = 24,
                gradient = { angle = 135, stops = { color, color .. "55" } },
                x = function() return math.sin(n:get() * speed) * 60 end,
                y = function() return math.cos(n:get() * speed) * 40 end,
            }
        end
        return ui.div { w = 500, h = 300, bg = "#120a2c",
            orb(40, 160, "#ff5a5f", 0.2), orb(180, 200, "#06d6a0", 0.13), orb(300, 140, "#4cc9f0", 0.17),
            ui.text { "GLASS", text_size = 90, weight = 900, color = "#ffffff", absolute = true, left = 90, top = 90 },
            ui.div {
                absolute = true, left = 30, top = 180, w = 200, h = 100, rounded = 20, backdrop_blur = 34,
                bg = "#ffffff22", y = function() return math.sin(n:get() * 0.1) * 20 end,
            },
            ui.div {
                absolute = true, top = 40, w = 120, h = 120, rounded = 60,
                left = function() return n:get() * 8 end,
                layer_shader = ui.layer_shader("fn layer(input: LayerInput) -> vec4<f32> { let b = zgx_backdrop(input.p + vec2(6.0, 0.0)); return vec4(b.rgb, 1.0); }"),
            },
        }
    "##;
    let frames = 40;
    let film = |partial: bool| {
        std::thread::spawn(move || {
            let mut stage = inline(script, 500., 300., 30.);
            let mut out = Vec::new();
            for _ in 0..frames {
                let mut pixels = Vec::new();
                if partial {
                    stage.draw_damaged(&mut pixels).unwrap();
                } else {
                    stage.draw(&mut pixels).unwrap();
                }
                out.push(pixels);
                stage.step().unwrap();
            }
            out
        })
    };
    let (partly, whole) = (film(true), film(false));
    let (partly, whole) = (partly.join().unwrap(), whole.join().unwrap());
    for (frame, (a, b)) in partly.iter().zip(&whole).enumerate() {
        let (count, first) = differ(a, b, 500);
        assert!(
            count == 0,
            "frame {frame}: {count} pixels differ, first at {first:?}"
        );
    }
}

#[test]
fn a_stage_leaves_its_thread_decoding_large_pictures_as_it_was() {
    // A stage made on an app's own thread (a still from a window's): its
    // frames decode every picture, and the thread's go back to what they were.
    assert!(!zgx::image::whole_frames_on_this_thread());
    let stage = inline("return ui.div { w = 10, h = 10 }", 10., 10., 30.);
    assert!(zgx::image::whole_frames_on_this_thread(), "while it lives");
    drop(stage);
    assert!(!zgx::image::whole_frames_on_this_thread(), "and back after");
    // Even when it fails to be made.
    let failed = zgx_video::Stage::new(Default::default(), |_| Err("no view".to_owned()));
    assert!(failed.is_err());
    assert!(!zgx::image::whole_frames_on_this_thread());
}
