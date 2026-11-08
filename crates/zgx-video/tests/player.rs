//! A composition playing inside an ordinary zgx view.
mod common;
use common::*;
use serde_json::json;
use std::time::{Duration, Instant};
use zgx::compose::prelude::*;
use zgx::scene::Color;
use zgx_video::{Player, Stage, StageOptions};

const SCRIPT: &str = r##"
local video = require("@video")
return video.compositions { video.composition {
    id = "Bar", w = 100, h = 50, fps = 30, duration = 60, props = { color = "#ff0000" },
    render = function(props)
        local f = video.frame()
        return ui.div { w_full = true, h_full = true, bg = "#000000",
            ui.div { h = 50, w = function() return f:get() end, bg = props.color } }
    end,
} }
"##;

#[test]
fn a_player_shows_its_compositions_frame_in_any_view() {
    let dir = temp_dir("player");
    std::fs::write(dir.join("video.luau"), SCRIPT).unwrap();
    let mut held = None;
    let path = dir.to_path_buf();
    let mut stage = Stage::new(
        StageOptions {
            width: 200.,
            height: 100.,
            fps: 60.,
            scale: 1.,
            background: Color(255, 255, 255, 255),
            ..Default::default()
        },
        |setting| {
            let player = Player::new(setting.runtime, &path, 1.)?;
            player.open("Bar", json!({ "color": "#00ff00" }), 40);
            let view = div().w(200.).h(100.).child(player.view().w(200.).h(100.));
            held = Some(player);
            Ok(view)
        },
    )
    .unwrap();
    let player = held.unwrap();
    // Its frames come from a thread of their own, in real time.
    let deadline = Instant::now() + Duration::from_secs(30);
    while player.frame.with_untracked(|f| *f) != 40 || player.image.with_untracked(Option::is_none)
    {
        stage.step().unwrap();
        assert!(Instant::now() < deadline, "no frame came");
        std::thread::sleep(Duration::from_millis(2));
    }
    stage.step().unwrap();
    let mut pixels = Vec::new();
    stage.draw(&mut pixels).unwrap();
    // 40 points of green bar in a 100-point frame shown 200 wide: 80 pixels.
    let green = (0..200u32)
        .filter(|x| pixel(&pixels, 200, *x, 50)[1] > 200 && pixel(&pixels, 200, *x, 50)[0] < 60)
        .count();
    assert!((76..=84).contains(&green), "{green} green pixels");
    assert_eq!(
        player.composition.with_untracked(|c| c.clone()).unwrap().id,
        "Bar"
    );
}

#[test]
fn a_script_plays_a_player_its_host_exposes() {
    let dir = temp_dir("player-luau");
    std::fs::write(dir.join("video.luau"), SCRIPT).unwrap();
    let mut held = None;
    let path = dir.to_path_buf();
    let mut stage = Stage::new(
        StageOptions {
            width: 100.,
            height: 50.,
            fps: 60.,
            scale: 1.,
            background: Color(255, 255, 255, 255),
            ..Default::default()
        },
        |setting| {
            let player = Player::new(setting.runtime, &path, 1.)?;
            let host = zgx_script::ScriptHost::inline(
                setting.runtime,
                "app",
                r##"
                local clip = ui.shared("clip", {} :: any)
                ui.on_mount(function()
                    ui.call("clip.listen", nil)
                    ui.invoke("clip.open", { id = "Bar", props = {}, frame = 25 })
                end)
                return ui.column { w = 100, h = 50,
                    ui.image { src = "host:clip", w = 100, h = 50, fit = "contain" } }
                "##,
            );
            host.lint_strip(false);
            player.expose(&host, "clip");
            host.reload()?;
            let view = host.view();
            held = Some((player, host));
            Ok(view)
        },
    )
    .unwrap();
    let (_player, host) = held.unwrap();
    let state = host.shared("clip", zgx_script::Value::Nil);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        stage.step().unwrap();
        let frame =
            serde_json::from_str::<serde_json::Value>(&state.with_untracked(|v| v.to_json()))
                .unwrap()["frame"]
                .clone();
        if frame == json!(25) {
            break;
        }
        assert!(Instant::now() < deadline, "no frame came");
        std::thread::sleep(Duration::from_millis(2));
    }
    stage.step().unwrap();
    let mut pixels = Vec::new();
    stage.draw(&mut pixels).unwrap();
    let red = (0..100u32)
        .filter(|x| pixel(&pixels, 100, *x, 25)[0] > 200 && pixel(&pixels, 100, *x, 25)[1] < 60)
        .count();
    assert!((23..=27).contains(&red), "{red} red pixels");
}

#[test]
fn a_stopped_player_tells_its_script_it_stopped() {
    // Stopping sends no frame: the script hears it all the same.
    let dir = temp_dir("player-stop");
    std::fs::write(dir.join("video.luau"), SCRIPT).unwrap();
    let mut held = None;
    let path = dir.to_path_buf();
    let mut stage = Stage::new(
        StageOptions {
            width: 100.,
            height: 50.,
            fps: 60.,
            scale: 1.,
            background: Color(255, 255, 255, 255),
            ..Default::default()
        },
        |setting| {
            let player = Player::new(setting.runtime, &path, 1.)?;
            let host = zgx_script::ScriptHost::inline(
                setting.runtime,
                "app",
                r##"
                local cmd = ui.shared("cmd", "")
                ui.on_mount(function()
                    ui.call("clip.listen", nil)
                    ui.invoke("clip.open", { id = "Bar", props = {}, frame = 0 })
                end)
                ui.effect(function()
                    local c = cmd:get()
                    if c ~= "" then ui.invoke("clip.play", c == "play") end
                end)
                return ui.column { w = 100, h = 50 }
                "##,
            );
            host.lint_strip(false);
            player.expose(&host, "clip");
            host.reload()?;
            let view = host.view();
            held = Some((player, host));
            Ok(view)
        },
    )
    .unwrap();
    let (_player, host) = held.unwrap();
    let state = host.shared("clip", zgx_script::Value::Nil);
    let read = |key: &str| {
        serde_json::from_str::<serde_json::Value>(&state.with_untracked(|v| v.to_json())).unwrap()
            [key]
            .clone()
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while read("frame") != json!(0) {
        stage.step().unwrap();
        assert!(Instant::now() < deadline, "no frame came");
        std::thread::sleep(Duration::from_millis(2));
    }
    let cmd = host.shared("cmd", zgx_script::Value::Nil);
    cmd.set(zgx_script::Value::from_json("\"play\"").unwrap());
    stage.step().unwrap();
    assert_eq!(read("playing"), json!(true));
    cmd.set(zgx_script::Value::from_json("\"stop\"").unwrap());
    stage.step().unwrap();
    assert_eq!(
        read("playing"),
        json!(false),
        "told it stopped, with no frame to tell it by"
    );
}
