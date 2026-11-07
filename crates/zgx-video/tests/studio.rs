//! The studio, headless: the app with its frames from their thread, in real time.
mod common;
use common::*;
use serde_json::json;
use std::rc::Rc;
use std::time::{Duration, Instant};
use zgx::scene::Color;
use zgx_script::ScriptHost;
use zgx_video::studio::{Studio, app_file, attach};
use zgx_video::{Stage, StageOptions};

const SCRIPT: &str = r##"
local video = require("@video")
return video.compositions { video.composition {
    id = "Bars", w = 64, h = 36, fps = 30, duration = 60, props = { width = 10 },
    render = function(props)
        local f = video.frame()
        return ui.div { w_full = true, h_full = true, bg = "#000000",
            ui.div { h = 10, w = function() return props.width + f:get() end, bg = "#ff0000" },
            video.sequence { from = 10, duration = 20, name = "Later", ui.div { w = 5, h = 5, bg = "#00ff00" } },
        }
    end,
} }
"##;
struct App {
    stage: Stage,
    host: ScriptHost,
    studio: Rc<Studio>,
}
fn app(script: &std::path::Path) -> App {
    // The app itself, with a way in for what a person presses.
    let source = std::fs::read_to_string(app_file().unwrap()).unwrap();
    let driver = r#"
local ask: any = ui.shared("test.ask", nil :: any)
ui.effect(function()
    local asked = ask:get()
    if asked then
        if asked.name == "studio.render" then
            ui.call(asked.name, asked.arg)
        else
            ui.invoke(asked.name, asked.arg)
        end
    end
end)
-- The composition to open as soon"#;
    assert!(source.contains("-- The composition to open as soon"));
    let source = source.replace("-- The composition to open as soon", driver);
    let app_dir = temp_dir("app");
    let app = app_dir.join("app.luau");
    std::fs::write(&app, source).unwrap();
    let mut held = None;
    let stage = Stage::new(
        StageOptions {
            width: 1000.,
            height: 700.,
            fps: 60.,
            scale: 1.,
            background: Color(0, 0, 0, 255),
            ..Default::default()
        },
        |setting| {
            let host = ScriptHost::new(setting.runtime, &app);
            host.lint_strip(false);
            let studio = attach(&host, script, 1.)?;
            studio.refresh(&json!({}));
            host.reload().map_err(|error| error.to_string())?;
            let view = host.view();
            held = Some((host, studio));
            Ok(view)
        },
    )
    .unwrap();
    let (host, studio) = held.unwrap();
    App {
        stage,
        host,
        studio,
    }
}
impl App {
    /// Run, in real time, until `done` holds of its state.
    fn until(
        &mut self,
        what: &str,
        done: impl Fn(&serde_json::Value) -> bool,
    ) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.stage.step().unwrap();
            let state = self.studio.state();
            if let Some(error) = self.host.error().with_untracked(Clone::clone) {
                panic!("{error}");
            }
            if done(&state) {
                return state;
            }
            assert!(Instant::now() < deadline, "no {what}: {state}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn invoke(&self, name: &str, arg: serde_json::Value) {
        let asked = json!({ "name": name, "arg": arg }).to_string();
        self.host
            .shared("test.ask", zgx_script::Value::Nil)
            .set(zgx_script::Value::from_json(&asked).unwrap());
    }
}

fn script_dir(name: &str) -> TempDir {
    let dir = temp_dir(name);
    std::fs::write(dir.join("video.luau"), SCRIPT).unwrap();
    dir
}

#[test]
fn it_lists_opens_and_shows_the_first_composition() {
    let dir = script_dir("open");
    let mut app = app(&dir);
    let state = app.until("first frame", |state| state["frame"].as_u64() == Some(0));
    assert_eq!(state["compositions"][0]["id"], "Bars");
    assert_eq!(state["open"]["duration"], 60);
    assert_eq!(state["size"], json!([64, 36]));
}

#[test]
fn a_frame_scrubbed_back_to_is_the_frame_a_render_writes() {
    let dir = script_dir("scrub");
    let mut app = app(&dir);
    app.until("first frame", |state| state["frame"].as_u64() == Some(0));
    app.invoke("studio.seek", json!(40));
    app.until("frame 40", |state| state["frame"].as_u64() == Some(40));
    app.invoke("studio.seek", json!(12));
    let state = app.until("frame 12", |state| state["frame"].as_u64() == Some(12));
    assert_eq!(state["tracks"][0]["name"], "Later", "{state}");
    let shown = app.studio.preview_pixels().expect("a preview");
    // On another thread, which has its own clock.
    let dir_there = dir.to_path_buf();
    let written = std::thread::spawn(move || {
        let mut stage = zgx_video::ScriptStage::new(
            &dir_there,
            None,
            &json!({}),
            1.,
            Color(0, 0, 0, 0),
            zgx_video::Timing::default(),
        )
        .unwrap();
        stage.step_to(12).unwrap();
        let mut pixels = Vec::new();
        stage.draw(&mut pixels).unwrap();
        pixels
    })
    .join()
    .unwrap();
    assert!(*shown == *written, "the preview and a render differ");
}

#[test]
fn it_plays_at_real_speed_and_stops_at_the_end_without_loop() {
    let dir = script_dir("play");
    let mut app = app(&dir);
    app.until("first frame", |state| state["frame"].as_u64() == Some(0));
    app.invoke("studio.loop", json!(false));
    let started = Instant::now();
    app.invoke("studio.play", json!(true));
    let state = app.until("the end", |state| state["playing"] == json!(false));
    let took = started.elapsed().as_secs_f64();
    assert!(took >= 1.9, "two seconds at real speed took {took}");
    assert_eq!(state["frame"], 59);
}

#[test]
fn new_props_show_at_once() {
    let dir = script_dir("props");
    let mut app = app(&dir);
    app.until("first frame", |state| state["frame"].as_u64() == Some(0));
    let reach = |pixels: &[u8]| {
        (0..64)
            .rev()
            .find(|x| pixel(pixels, 64, *x, 8)[0] > 128)
            .map_or(0, |x| x + 1)
    };
    assert_eq!(reach(&app.studio.preview_pixels().unwrap()), 10);
    app.invoke(
        "studio.open",
        json!({ "id": "Bars", "props": { "width": 30 }, "frame": 0 }),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while reach(&app.studio.preview_pixels().unwrap()) != 30 {
        app.stage.step().unwrap();
        assert!(Instant::now() < deadline, "the props never showed");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_broken_script_shows_its_error_and_a_fixed_one_comes_back() {
    let dir = script_dir("reload");
    let mut app = app(&dir);
    app.until("first frame", |state| state["frame"].as_u64() == Some(0));
    std::fs::write(dir.join("video.luau"), "this is not luau").unwrap();
    app.studio.reload();
    app.until("an error", |state| state["error"].is_string());
    std::fs::write(dir.join("video.luau"), SCRIPT.replace("#ff0000", "#0000ff")).unwrap();
    app.studio.reload();
    app.until("its fix", |state| {
        state["error"].is_null() && state["frame"].as_u64() == Some(0)
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    while pixel(&app.studio.preview_pixels().unwrap(), 64, 2, 2)[2] < 200 {
        app.stage.step().unwrap();
        assert!(Instant::now() < deadline, "the fix never showed");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn its_renders_queue_run_in_turn_and_a_queued_one_can_be_stopped() {
    let dir = script_dir("render");
    let mut app = app(&dir);
    app.until("first frame", |state| state["frame"].as_u64() == Some(0));
    for name in ["one", "two", "three"] {
        app.invoke(
            "studio.render",
            json!({ "id": "Bars", "props": {}, "output": dir.join(name) }),
        );
        app.stage.step().unwrap();
    }
    // The third waits behind the others: stop it before its turn.
    app.until("three queued", |state| {
        state["renders"]
            .as_array()
            .is_some_and(|jobs| jobs.len() == 3)
    });
    app.invoke("studio.cancel", json!(3));
    let state = app.until("the queue run", |state| {
        state["renders"].as_array().is_some_and(|jobs| {
            jobs.iter()
                .all(|job| matches!(job["state"].as_str(), Some("done" | "stopped")))
        })
    });
    let states: Vec<&str> = state["renders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|job| job["state"].as_str().unwrap())
        .collect();
    assert_eq!(states, ["done", "done", "stopped"], "{state}");
    assert_eq!(std::fs::read_dir(dir.join("one")).unwrap().count(), 60);
    assert_eq!(std::fs::read_dir(dir.join("two")).unwrap().count(), 60);
    assert!(!dir.join("three").exists(), "a stopped one never started");
}
