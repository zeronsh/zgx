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

#[test]
fn a_players_translucent_picture_shows_over_what_is_behind_it_as_drawn() {
    let dir = temp_dir("player-translucent");
    // Half-transparent red over nothing: its frame, premultiplied, is
    // (128, 0, 0, 128).
    std::fs::write(
        dir.join("video.luau"),
        r##"
        local video = require("@video")
        return video.compositions { video.composition {
            id = "Half", w = 100, h = 50, fps = 30, duration = 10,
            render = function()
                return ui.div { w_full = true, h_full = true, bg = "#ff000080" }
            end,
        } }
        "##,
    )
    .unwrap();
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
            player.open("Half", json!({}), 0);
            let view = div().w(100.).h(50.).child(player.view().w(100.).h(50.));
            held = Some(player);
            Ok(view)
        },
    )
    .unwrap();
    let player = held.unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while player.image.with_untracked(Option::is_none) {
        stage.step().unwrap();
        assert!(Instant::now() < deadline, "no frame came");
        std::thread::sleep(Duration::from_millis(2));
    }
    stage.step().unwrap();
    let mut pixels = Vec::new();
    stage.draw(&mut pixels).unwrap();
    // Half red over white: (255, 127, 127). Premultiplied twice, its red
    // fell to about 191.
    let [r, g, b, _] = pixel(&pixels, 100, 50, 25);
    assert!(
        r >= 250 && (124..=131).contains(&g) && (124..=131).contains(&b),
        "{r} {g} {b}"
    );
}

/// Each frame a different bar, so a picture tells its frame.
const RUN: &str = r##"
local video = require("@video")
return video.compositions { video.composition {
    id = "Run", w = 100, h = 50, fps = 30, duration = DURATION,
    render = function()
        local f = video.frame()
        return ui.div { w_full = true, h_full = true, bg = "#000000",
            ui.div { h = 50, w = function() return f:get() % 100 end, bg = "#00ff00" } }
    end,
} }
"##;

/// A player shown in a 200 by 100 stage (`placed` puts its box), stepped
/// until `done` says so.
struct Shown {
    stage: Stage,
    player: std::rc::Rc<Player>,
}
fn shown(dir: &std::path::Path, placed: impl Fn(View) -> View) -> Shown {
    let mut held = None;
    let path = dir.to_path_buf();
    let stage = Stage::new(
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
            let view = div()
                .w(200.)
                .h(100.)
                .overflow_hidden()
                .child(placed(player.view().w(100.).h(50.)));
            held = Some(player);
            Ok(view)
        },
    )
    .unwrap();
    Shown {
        stage,
        player: held.unwrap(),
    }
}
impl Shown {
    fn frame(&self) -> Option<u64> {
        self.player
            .image
            .with_untracked(Option::is_some)
            .then(|| self.player.frame.with_untracked(|f| *f))
    }
    fn until(&mut self, what: impl Fn(Option<u64>) -> bool) -> Option<u64> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.stage.step().unwrap();
            if what(self.frame()) {
                return self.frame();
            }
            assert!(
                Instant::now() < deadline,
                "it never came: {:?}",
                self.frame()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    /// Steps for `time`, returning the frames seen meanwhile.
    fn watch_for(&mut self, time: Duration) -> Vec<Option<u64>> {
        let until = Instant::now() + time;
        let mut seen = Vec::new();
        while Instant::now() < until {
            self.stage.step().unwrap();
            seen.push(self.frame());
            std::thread::sleep(Duration::from_millis(5));
        }
        seen
    }
    fn pixels(&self) -> Vec<u8> {
        self.player
            .image
            .with_untracked(|image| image.as_ref().unwrap().pixels().to_vec())
    }
}

#[test]
fn an_unseen_player_draws_nothing_and_seen_again_shows_the_frame_due_then() {
    let dir = temp_dir("player-unseen");
    std::fs::write(dir.join("video.luau"), RUN.replace("DURATION", "900")).unwrap();
    let mut shown = shown(&dir, |view| view);
    shown.player.open("Run", json!({}), 0);
    shown.player.play(true);
    shown.until(|frame| frame.is_some_and(|f| f >= 10));
    shown.player.watch(false);
    let hidden = Instant::now();
    // A frame on its way as it was told may still come; then none.
    shown.watch_for(Duration::from_millis(100));
    let held = shown.frame();
    let meanwhile = shown.watch_for(Duration::from_millis(600));
    assert!(
        meanwhile.iter().all(|f| *f == held),
        "drew unseen: {meanwhile:?}"
    );
    // Seen again: the frame its clock is at, not the one it left.
    let before = held.unwrap();
    let seen_at = Instant::now();
    shown.player.watch(true);
    let resumed = shown.until(|frame| frame != held).unwrap();
    let elapsed = |at: Instant| (at - hidden).as_secs_f64() * 30.;
    let (low, high) = (elapsed(seen_at) - 2., elapsed(Instant::now()) + 2.);
    let ran = (resumed - before) as f64;
    assert!(
        ran >= low && ran <= high + 4.,
        "resumed at {resumed}, {before} + {ran} (due {low}..{high})"
    );
    // And the picture is that frame's, as a player opened there draws it.
    let resumed_pixels = shown.pixels();
    drop(shown);
    let mut fresh = self::shown(&dir, |view| view);
    fresh.player.open("Run", json!({}), resumed);
    fresh.until(|frame| frame == Some(resumed));
    assert!(
        fresh.pixels() == resumed_pixels,
        "frame {resumed} drawn differently"
    );
}

#[test]
fn a_looping_player_unseen_past_its_end_resumes_where_time_has_it() {
    let dir = temp_dir("player-unseen-loop");
    // One second a pass.
    std::fs::write(dir.join("video.luau"), RUN.replace("DURATION", "30")).unwrap();
    let mut shown = shown(&dir, |view| view);
    shown.player.open("Run", json!({}), 0);
    shown.player.looping(true);
    shown.player.play(true);
    let before = shown.until(|frame| frame.is_some_and(|f| f >= 3)).unwrap();
    shown.player.watch(false);
    let hidden = Instant::now();
    shown.watch_for(Duration::from_millis(1500));
    shown.player.watch(true);
    let seen_at = Instant::now();
    let held = shown.frame();
    let resumed = shown.until(|frame| frame != held).unwrap();
    // Half a pass on from where it was: not back at its start.
    let due = (before as f64 + (seen_at - hidden).as_secs_f64() * 30.) % 30.;
    let off = ((resumed as f64 - due + 45.) % 30.) - 15.;
    assert!(off.abs() <= 4., "resumed at {resumed}, due about {due:.1}");
}

#[test]
fn a_player_whose_box_is_clipped_away_draws_nothing() {
    let dir = temp_dir("player-offscreen");
    std::fs::write(dir.join("video.luau"), RUN.replace("DURATION", "900")).unwrap();
    // Its box outside the clip of the stage's box: none of it shows.
    let mut shown = shown(&dir, |view| view.absolute().left(400.));
    shown.player.open("Run", json!({}), 40);
    shown.player.play(true);
    let meanwhile = shown.watch_for(Duration::from_millis(500));
    assert!(
        meanwhile.iter().all(Option::is_none),
        "drew offscreen: {meanwhile:?}"
    );
}

#[test]
fn a_player_unseen_past_its_end_shows_its_last_frame_stopped() {
    let dir = temp_dir("player-unseen-end");
    // One second, not looping.
    std::fs::write(dir.join("video.luau"), RUN.replace("DURATION", "30")).unwrap();
    let mut shown = shown(&dir, |view| view);
    shown.player.open("Run", json!({}), 0);
    shown.player.looping(false);
    shown.player.play(true);
    shown.until(|frame| frame.is_some_and(|f| f >= 3));
    shown.player.watch(false);
    // Past its end while unseen: it stops there, drawing nothing.
    let deadline = Instant::now() + Duration::from_secs(30);
    while shown_playing(&shown) {
        shown.stage.step().unwrap();
        assert!(Instant::now() < deadline, "it never stopped");
        std::thread::sleep(Duration::from_millis(5));
    }
    let held = shown.frame();
    let meanwhile = shown.watch_for(Duration::from_millis(300));
    assert!(
        meanwhile.iter().all(|f| *f == held),
        "drew unseen: {meanwhile:?}"
    );
    // Seen again: its last frame, and still stopped.
    shown.player.watch(true);
    let last = shown.until(|frame| frame == Some(29));
    assert_eq!(last, Some(29));
    assert!(!shown_playing(&shown), "playing again");
    let after = shown.watch_for(Duration::from_millis(200));
    assert!(after.iter().all(|f| *f == Some(29)), "moved on: {after:?}");
}
fn shown_playing(shown: &Shown) -> bool {
    shown.player.playing.with_untracked(|p| *p)
}
