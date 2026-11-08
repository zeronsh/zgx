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

/// A one-shot in the gallery fires where it's pressed, in its box's points.
#[test]
fn the_gallerys_one_shots_fire_where_pressed() {
    let gallery = include_str!("../examples/gallery/app.luau").replace("../../luau/fx", "./fx");
    let mut app = app(&gallery);
    app.ui.scene.borrow_mut().resize(900., 1200.);
    app.frames(3);
    let ripple = app.find("ripple").unwrap();
    let size = app.bounds("ripple");
    app.click("ripple");
    app.frames(2);
    assert_eq!(app.host.error().get(), None);
    let scene = app.ui.scene.borrow();
    // The first shot's row: where, in points from the box's top left.
    let shot = scene.layer_shader(ripple).unwrap().uniforms.params[1];
    let middle = (size.width / 2., size.height / 2.);
    assert!(
        (shot[0] - middle.0).abs() < 1. && (shot[1] - middle.1).abs() < 1.,
        "fired at {:?}, pressed at {middle:?}",
        (shot[0], shot[1])
    );
}

/// The gallery's trail and bloom are passes reading moving boxes, as an app
/// writes them: the renderer takes their shaders, and drawn over a few frames
/// the trail leaves light beside its boxes and the bloom glows past their edges.
#[test]
fn the_gallerys_trail_and_bloom_draw_light_from_their_boxes() {
    let gallery = include_str!("../examples/gallery/app.luau").replace("../../luau/fx", "./fx");
    let mut app = app(&gallery);
    let (w, h) = (900u32, 1200u32);
    // The window's size, the cells in view.
    app.ui.scene.borrow_mut().resize(w as f32, h as f32);
    app.frames(3);
    assert_eq!(app.host.error().get(), None);
    let mut gpu = zgx_gpu::GpuRenderer::new(w, h).unwrap();
    gpu.set_background(zgx::scene::Color(0, 0, 0, 255));
    // Its passes compiled before the frames that read them, as a window's
    // are by the time anyone looks.
    gpu.debug_wait_for_shaders();
    let mut pixels = Vec::new();
    for _ in 0..40 {
        app.frames(1);
        let scene = app.ui.scene.borrow();
        let whole = zgx::scene::Rect::new(0., 0., w as f32, h as f32);
        gpu.render(&scene, &[whole]).unwrap();
        pixels = gpu.readback().unwrap();
    }
    if let Ok(out) = std::env::var("ZGX_PASSES_OUT") {
        image::save_buffer(&out, &pixels, w, h, image::ExtendedColorType::Rgba8).unwrap();
    }
    let nodes = ["trail", "bloom"].map(|name| (name, app.find(name).unwrap()));
    let scene = app.ui.scene.borrow();
    let bright = |x: f32, y: f32| -> u32 {
        let at = ((y as u32 * w + x as u32) * 4) as usize;
        pixels[at..at + 3].iter().map(|v| u32::from(*v)).sum()
    };
    // The panel's own colour, #10131c.
    let panel = 0x10 + 0x13 + 0x1c;
    for (name, node) in nodes {
        let shader = scene
            .layer_shader(node)
            .and_then(|l| l.shader.clone())
            .unwrap();
        let inputs = shader.inputs().expect("passes");
        assert_eq!(
            zgx_gpu::check_layer_inputs(shader.source(), inputs),
            Ok(()),
            "{name}"
        );
        let mut boxes = Vec::new();
        scene.geometry_of(inputs.geometry.unwrap(), &mut boxes);
        assert_eq!(boxes.len(), 3, "{name}: its boxes");
        let (b, _) = boxes[1];
        let near = match name {
            // Beside the box, either way it was going: where it just was.
            "trail" => bright(b.x - 6., b.y + b.height / 2.)
                .max(bright(b.x + b.width + 6., b.y + b.height / 2.)),
            // Just above its top edge.
            _ => bright(b.x + b.width / 2., b.y - 3.),
        };
        assert!(
            near > panel + 30,
            "{name}: {near} by its box, the panel {panel}"
        );
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
