//! The Luau module: its shaders and numbers are the Rust side's, and the gallery
//! runs with every effect through ordinary props.
use zgx_fx::{NAMES, catalog};
use zgx_script::ScriptHost;
use zgx_script::testing::TestApp;

const MODULE: &str = include_str!("../luau/fx.luau");

#[test]
fn it_holds_each_effects_shader_as_rust_does() {
    for (name, _, source) in zgx_fx::sources() {
        assert!(MODULE.contains(source), "{name}: run scripts/fx_luau.py");
    }
}

/// A script run from a directory of its own, the module beside it.
fn app(code: &str) -> TestApp {
    let dir = std::env::temp_dir().join(format!(
        "zgx-fx-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("fx.luau"), MODULE).unwrap();
    let entry = dir.join("app.luau");
    std::fs::write(&entry, code).unwrap();
    TestApp::new(move |runtime| ScriptHost::new(runtime, entry))
}

#[test]
fn its_numbers_are_rusts() {
    let list = NAMES
        .iter()
        .map(|name| {
            format!(
                r#"(function() local e = fx.{name}() return ui.text {{ id = "{name}", tostring(e.bleed) .. "," .. tostring(e.reach) .. "," .. tostring(e.frost) .. "," .. tostring(e.running) }} end)(),"#
            )
        })
        .collect::<String>();
    let mut app = app(&format!(
        r#"local fx = require("./fx")
        return ui.column {{ {list} }}"#
    ));
    app.frames(2);
    assert_eq!(app.host.error().get(), None);
    for (name, fx) in catalog() {
        if name.contains(' ') {
            continue;
        }
        let shown = app.text(&name);
        let parts: Vec<&str> = shown.split(',').collect();
        let close = |text: &str, value: f32| {
            let read: f32 = text.parse().unwrap();
            assert!((read - value).abs() < 1e-3, "{name}: {shown}");
        };
        close(parts[0], fx.bleed());
        close(parts[1], fx.reach());
        close(parts[2], fx.frost());
        assert_eq!(parts[3], fx.is_animated().to_string(), "{name}");
    }
}

#[test]
fn the_gallery_runs_and_a_one_shot_goes_away() {
    let gallery = include_str!("../examples/gallery/app.luau").replace("../../luau/fx", "./fx");
    let mut app = app(&gallery);
    app.frames(3);
    assert_eq!(app.host.error().get(), None);
    assert!(app.find("melt").is_some());
    let melt = app.find("melt").unwrap();
    assert!(app.ui.scene.borrow().layer_shader(melt).is_some());
    // A picture of it melted, asked for.
    if let Ok(out) = std::env::var("ZGX_MELT_OUT") {
        let scene = app.ui.scene.clone();
        let mut scene = scene.borrow_mut();
        let mut melted = scene.layer_shader(melt).unwrap().uniforms;
        melted.time = 1.9;
        scene.set_layer_uniforms(melt, melted);
        let _ = scene.flush();
        let (w, h) = (880u32, 600u32);
        let mut gpu = zgx_gpu::GpuRenderer::new(w, h).unwrap();
        gpu.set_scale_factor(2.);
        gpu.set_background(zgx::scene::Color(12, 14, 20, 255));
        let whole = zgx::scene::Rect::new(0., 0., w as f32 / 2., h as f32 / 2.);
        gpu.render(&scene, &[whole]).unwrap();
        let pixels = gpu.readback().unwrap();
        image::save_buffer(&out, &pixels, w, h, image::ExtendedColorType::Rgba8).unwrap();
    }
}

/// A moving effect takes no script as it moves, and a one-shot none once fired.
#[test]
fn what_moves_runs_no_script_per_frame() {
    let mut app = app(r#"local fx = require("./fx")
        local ripple = fx.wave()
        return ui.column {
            ui.div(fx.with(fx.pulse(), { id = "pulse", w = 100, h = 60 })),
            ui.button(fx.with(ripple, { id = "wave", w = 100, h = 60,
                on_click = function() ripple:fire(10, 10) end, "Fire" })),
        }"#);
    app.frames(3);
    assert_eq!(app.host.error().get(), None);
    let wave = app.find("wave").unwrap();
    assert!(
        app.ui
            .scene
            .borrow()
            .layer_shader(wave)
            .is_none_or(|l| l.shader.is_none()),
        "no shot alive, no shader"
    );
    let calls = app.calls();
    app.frames(30);
    assert_eq!(app.calls(), calls, "a pulse steps with no script");
    app.click("wave");
    app.frames(2);
    assert_eq!(app.host.error().get(), None);
    let live = app
        .ui
        .scene
        .borrow()
        .layer_shader(wave)
        .is_some_and(|l| l.shader.is_some());
    assert!(live, "fired, it draws");
    let calls = app.calls();
    app.frames(10);
    assert_eq!(app.calls(), calls, "a shot steps with no script");
}
