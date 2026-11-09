//! `ui.scene3d`: a 3D scene in a box, drawn only when what it reads
//! changes, its bindings stepped in Rust with no script per frame.
mod common;
use common::Harness;
use zgx::scene::{NodeId, NodeKind};

/// The image a GPU view's box shows, its frame (box, then its clip, then
/// the frame).
fn frame_id(h: &mut Harness, id: &str) -> Option<u64> {
    let root = h.find(id)?;
    let scene = h.ui.scene.borrow();
    let clip = *scene.children(root).first()?;
    let frame: NodeId = *scene.children(clip).first()?;
    match scene.kind(frame) {
        NodeKind::Image(image) if image.gpu_frame().is_some() => Some(image.id()),
        _ => None,
    }
}

const SCENE: &str = r##"
local angle = ui.state("angle", 0)
local red = ui.state("red", false)
return ui.column {
    ui.button { id = "turn", w = 40, h = 20, on_click = function() angle:set(angle:get() + 90) end, "turn" },
    ui.button { id = "paint", w = 40, h = 20, on_click = function() red:set(not red:get()) end, "paint" },
    ui.text { id = "label", "a label" },
    ui.scene3d {
        id = "scene",
        w = 200,
        h = 100,
        background = "#101418",
        camera = { position = { 0, 1, 4 }, fov = 40 },
        ui.light { kind = "directional", direction = { -1, -2, -1 }, intensity = 3 },
        ui.light { kind = "hemisphere", color = "#8899aa", ground = "#221a14" },
        ui.group {
            position = { 0, 0.5, 0 },
            ui.mesh {
                geometry = "box",
                rotation = { 0, ui.spring(angle), 0 },
                material = { color = function() return if red:get() then "#ff0000" else "#3366ff" end, roughness = 0.4 },
            },
            ui.mesh { geometry = "sphere", radius = 0.3, position = { 1.2, 0, 0 } },
        },
        ui.mesh { geometry = "plane", size = { 6, 1, 6 }, position = { 0, -0.5, 0 } },
    },
}
"##;

#[test]
fn a_scene_shows_in_a_gpu_view_box() {
    let mut h = Harness::inline(SCENE);
    assert_eq!(h.error(), None);
    h.frame();
    assert!(frame_id(&mut h, "scene").is_some(), "{}", h.tree());
    assert_eq!(h.bounds("scene").width, 200.);
}

#[test]
fn a_still_scene_draws_no_frame() {
    let mut h = Harness::inline(SCENE);
    h.frames(3);
    let still = frame_id(&mut h, "scene");
    let calls = h.calls();
    h.frames(30);
    assert_eq!(frame_id(&mut h, "scene"), still, "nothing changed");
    assert_eq!(h.calls(), calls, "no script ran");
}

#[test]
fn a_bound_state_draws_new_frames_without_script_per_frame() {
    let mut h = Harness::inline(SCENE);
    h.frames(3);
    let still = frame_id(&mut h, "scene");
    // A spring toward the new angle: frames while it moves, from Rust.
    h.click("turn");
    let calls = h.calls();
    let mut frames = std::collections::HashSet::new();
    for _ in 0..30 {
        h.frame();
        frames.insert(frame_id(&mut h, "scene"));
    }
    assert!(
        frames.len() > 5,
        "a frame each step of the spring: {}",
        frames.len()
    );
    assert_ne!(frame_id(&mut h, "scene"), still);
    assert_eq!(h.calls(), calls, "no script ran while the spring moved");
    // Settled, it draws no more.
    h.frames(300);
    let settled = frame_id(&mut h, "scene");
    h.frames(10);
    assert_eq!(frame_id(&mut h, "scene"), settled, "idle once settled");
}

#[test]
fn a_colour_read_from_state_draws_a_frame_when_it_changes() {
    let mut h = Harness::inline(SCENE);
    h.frames(3);
    let before = frame_id(&mut h, "scene");
    h.click("paint");
    h.frame();
    assert_ne!(frame_id(&mut h, "scene"), before);
    let after = frame_id(&mut h, "scene");
    h.frames(5);
    assert_eq!(frame_id(&mut h, "scene"), after);
}

/// What `make` raises, shown as text.
fn raised(make: &str) -> String {
    let mut h = Harness::inline(&format!(
        r#"local ok, err = pcall(function() return {make} end)
return ui.text {{ id = "error", if ok then "no error" else tostring(err) }}"#
    ));
    h.text("error")
}

#[test]
fn objects_and_props_are_checked() {
    assert!(raised(r#"ui.mesh { geometry = "teapot" }"#).contains("geometry"));
    assert!(raised(r#"ui.scene3d { ui.text { "not 3D" } }"#).contains("3D objects"));
    assert!(raised(r#"ui.mesh { position = { "x" } }"#).contains("position"));
    assert!(raised(r#"ui.light { kind = "laser" }"#).contains("kind"));
    assert!(raised(r#"ui.scene3d { orbit = 3 }"#).contains("orbit"));
    assert_eq!(
        raised(r#"ui.mesh { geometry = "torus", scale = 2 }"#),
        "no error"
    );
}

const PICKED: &str = r##"
local clicked = ui.state("clicked", "none")
local over = ui.state("over", false)
local group = ui.state("group", 0)
return ui.column {
    ui.text { id = "clicked", "{clicked}" },
    ui.text { id = "over", function() return if over:get() then "over" else "out" end },
    ui.text { id = "group", "{group}" },
    ui.scene3d {
        id = "scene",
        w = 200,
        h = 200,
        orbit = true,
        camera = { position = { 0, 0, 4 }, fov = 40 },
        ui.mesh {
            geometry = "box",
            on_click = function(hit) clicked:set(string.format("%.2f", hit.z)) end,
            on_hover = function(is_over) over:set(is_over) end,
        },
        ui.group {
            position = { 0, 1.2, 0 },
            on_click = function() group:set(group:get() + 1) end,
            ui.mesh { geometry = "sphere", radius = 0.3 },
        },
    },
}
"##;

/// Press and release at `(x, y)` of the box `id`, from its top left, the
/// release `moved` points further right.
fn press(h: &mut zgx_script::testing::TestApp, id: &str, (x, y): (f32, f32), moved: f32) {
    use zgx::input::{InputEvent, PointerButton};
    let b = h.bounds(id);
    let (x, y) = (b.x + x, b.y + y);
    let button = PointerButton::Primary;
    h.ui.dispatch(InputEvent::PointerDown { x, y, button });
    h.ui.dispatch(InputEvent::PointerMove { x: x + moved, y });
    h.ui.dispatch(InputEvent::PointerUp {
        x: x + moved,
        y,
        button,
    });
    h.settle();
}

#[test]
fn a_click_on_a_mesh_says_where_and_one_beside_it_nothing() {
    let mut h = Harness::inline(PICKED);
    assert_eq!(h.error(), None);
    h.frame();
    // The box's front face, half a unit towards the camera.
    press(&mut h, "scene", (100., 100.), 0.);
    assert_eq!(h.text("clicked"), "0.50");
    h.ui.dispatch(zgx::input::InputEvent::PointerMove { x: 0., y: 0. });
    press(&mut h, "scene", (5., 195.), 0.);
    assert_eq!(h.text("clicked"), "0.50", "nothing there");
}

#[test]
fn a_drag_orbits_and_clicks_nothing() {
    let mut h = Harness::inline(PICKED);
    h.frame();
    press(&mut h, "scene", (100., 100.), 30.);
    assert_eq!(h.text("clicked"), "none");
}

#[test]
fn a_click_on_a_mesh_in_a_group_reaches_the_group() {
    let mut h = Harness::inline(PICKED);
    h.frame();
    // The sphere, 1.2 up: about 0.82 of the half height at fov 40, 4 away.
    let up = 1.2 / (4. * 20_f32.to_radians().tan());
    press(&mut h, "scene", (100., 100. - 100. * up), 0.);
    assert_eq!(h.text("group"), "1");
    assert_eq!(h.text("clicked"), "none");
}

#[test]
fn hovering_says_over_and_out() {
    let mut h = Harness::inline(PICKED);
    h.frame();
    let b = h.bounds("scene");
    let at = |x: f32, y: f32| zgx::input::InputEvent::PointerMove {
        x: b.x + x,
        y: b.y + y,
    };
    h.ui.dispatch(at(100., 100.));
    h.settle();
    assert_eq!(h.text("over"), "over");
    h.ui.dispatch(at(5., 195.));
    h.settle();
    assert_eq!(h.text("over"), "out");
    h.ui.dispatch(at(100., 100.));
    h.ui.dispatch(zgx::input::InputEvent::PointerMove { x: -50., y: -50. });
    h.settle();
    assert_eq!(h.text("over"), "out", "leaving the box");
}

#[test]
fn a_handler_must_be_a_function() {
    assert!(raised(r#"ui.scene3d { ui.mesh { on_click = 3 } }"#).contains("on_click"));
}

#[test]
fn shadows_are_asked_for_with_true() {
    let mut h = Harness::inline(
        r#"return ui.scene3d { id = "scene", w = 100, h = 100, shadows = true,
            ui.light { kind = "directional" }, ui.mesh { geometry = "box" } }"#,
    );
    assert_eq!(h.error(), None);
    h.frame();
    assert!(frame_id(&mut h, "scene").is_some());
    assert!(raised(r#"ui.scene3d { shadows = 1 }"#).contains("shadows"));
}

#[test]
fn an_environment_is_named_given_or_turned_off() {
    for environment in [
        r#""studio""#,
        r#""none""#,
        r#"{ intensity = 1.5, rotation = 90 }"#,
        r#"{ src = "studio", intensity = function() return 2 end }"#,
    ] {
        let mut h = Harness::inline(&format!(
            r#"return ui.scene3d {{ id = "scene", w = 100, h = 100, environment = {environment},
                ui.mesh {{ geometry = "sphere", material = {{ metallic = 1, roughness = 0.2 }} }} }}"#
        ));
        assert_eq!(h.error(), None, "{environment}");
        h.frame();
        assert!(frame_id(&mut h, "scene").is_some(), "{environment}");
    }
    assert!(raised(r#"ui.scene3d { environment = 3 }"#).contains("environment"));
    // A file that isn't there is an error the script sees.
    let h = Harness::inline(
        r#"return ui.scene3d { id = "scene", w = 100, h = 100, environment = "no such sky.hdr",
            ui.mesh { geometry = "box" } }"#,
    );
    h.frame();
    let error = h.error().unwrap_or_default();
    assert!(error.contains("no such sky.hdr"), "{error}");
}

/// A `.glb` of a unit square facing +z, as glTF writes one, with a clip
/// ("slide") moving it from 0 to 4 along x over 4 seconds.
#[cfg(feature = "gltf")]
fn square_glb() -> Vec<u8> {
    let positions: [f32; 12] = [-0.5, -0.5, 0., 0.5, -0.5, 0., 0.5, 0.5, 0., -0.5, 0.5, 0.];
    let mut bin: Vec<u8> = positions.iter().flat_map(|v| v.to_le_bytes()).collect();
    bin.extend([0_u16, 1, 2, 0, 2, 3].iter().flat_map(|i| i.to_le_bytes()));
    let keys: [f32; 8] = [0., 4., 0., 0., 0., 4., 0., 0.];
    bin.extend(keys.iter().flat_map(|v| v.to_le_bytes()));
    let mut json = format!(
        r#"{{"asset":{{"version":"2.0"}},"buffers":[{{"byteLength":{}}}],
        "bufferViews":[{{"buffer":0,"byteLength":48}},{{"buffer":0,"byteOffset":48,"byteLength":12}},
            {{"buffer":0,"byteOffset":60,"byteLength":8}},{{"buffer":0,"byteOffset":68,"byteLength":24}}],
        "accessors":[{{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3"}},
            {{"bufferView":1,"componentType":5123,"count":6,"type":"SCALAR"}},
            {{"bufferView":2,"componentType":5126,"count":2,"type":"SCALAR"}},
            {{"bufferView":3,"componentType":5126,"count":2,"type":"VEC3"}}],
        "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"indices":1}}]}}],
        "animations":[{{"name":"slide","samplers":[{{"input":2,"output":3}}],
            "channels":[{{"sampler":0,"target":{{"node":0,"path":"translation"}}}}]}}],
        "nodes":[{{"mesh":0}}],"scenes":[{{"nodes":[0]}}]}}"#,
        bin.len()
    )
    .into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut out = b"glTF".to_vec();
    out.extend(2_u32.to_le_bytes());
    out.extend(((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
    out.extend((json.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(json);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(bin);
    out
}

/// A script `code` beside `square.glb`, in a directory of its own.
#[cfg(feature = "gltf")]
fn beside_a_model(name: &str, code: &str) -> Harness {
    let dir = std::env::temp_dir().join(format!("zgx-model-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("square.glb"), square_glb()).unwrap();
    std::fs::write(dir.join("app.luau"), code).unwrap();
    Harness::new(|runtime| zgx_script::ScriptHost::new(runtime, dir.join("app.luau")))
}

#[cfg(feature = "gltf")]
#[test]
fn a_model_shows_once_read_and_hears_clicks_on_its_meshes() {
    let mut h = beside_a_model(
        "click",
        r#"
local clicked = ui.state("clicked", "none")
return ui.column {
    ui.text { id = "clicked", "{clicked}" },
    ui.scene3d {
        id = "scene", w = 200, h = 200,
        camera = { position = { 0, 0, 3 } },
        ui.model { src = "square.glb", fit = 2, on_click = function(hit) clicked:set(string.format("%.1f", hit.x)) end },
    },
}
"#,
    );
    assert_eq!(h.error(), None);
    // Read off the main thread: it shows (and hears) once read.
    let heard = h.wait_until(|h| {
        h.frame();
        press(h, "scene", (100., 100.), 0.);
        h.text("clicked") != "none"
    });
    assert!(heard, "never clicked: {}", h.tree());
    assert_eq!(h.text("clicked"), "0.0");
}

#[cfg(feature = "gltf")]
#[test]
fn a_model_that_cannot_be_read_says_so() {
    let h = beside_a_model(
        "missing",
        r#"return ui.scene3d { w = 100, h = 100, ui.model { src = "nowhere.glb" } }"#,
    );
    h.frame();
    let error = h.error().unwrap_or_default();
    assert!(error.contains("nowhere.glb"), "{error}");
    assert!(raised(r#"ui.scene3d { ui.model { fit = 2 } }"#).contains("src"));
    // Named alone (a module building objects), it is still there.
    assert!(raised(r#"ui.model { fit = 2 }"#).contains("src"));
}

#[cfg(not(feature = "gltf"))]
#[test]
fn a_model_without_the_gltf_feature_says_so() {
    assert!(raised(r#"ui.scene3d { ui.model { src = "a.glb" } }"#).contains("`gltf` feature"));
}

#[cfg(feature = "gltf")]
#[test]
fn a_model_plays_its_clip_frame_by_frame() {
    // Sliding along x a unit a second; the camera 6 back sees x 1 at 40
    // points right of the middle (its half width 2.49 at that distance).
    let mut h = beside_a_model(
        "play",
        r#"
local clicks = ui.state("clicks", 0)
local at = ui.state("at", "none")
return ui.column {
    ui.text { id = "clicks", "{clicks}" },
    ui.text { id = "at", "{at}" },
    ui.scene3d {
        id = "scene", w = 200, h = 200,
        camera = { position = { 0, 0, 6 } },
        ui.model {
            src = "square.glb",
            animation = "slide",
            on_click = function(hit)
                clicks:set(clicks:get() + 1)
                at:set(string.format("%.1f", hit.x))
            end,
        },
    },
}
"#,
    );
    let px = |x: f32| (100. + x / (6. * 22.5_f32.to_radians().tan()) * 100., 100.);
    // Read, and where it starts (x 0.3 on it).
    assert!(
        h.wait_until(|h| {
            h.frame();
            press(h, "scene", px(0.3), 0.);
            h.text("clicks") != "0"
        }),
        "never read"
    );
    // A second on: a unit along, past that point, onto x 1.3.
    h.frames(60);
    let clicked = h.text("clicks");
    press(&mut h, "scene", px(0.3), 0.);
    assert_eq!(h.text("clicks"), clicked, "it slid away");
    press(&mut h, "scene", px(1.3), 0.);
    assert_ne!(h.text("clicks"), clicked);
    assert_eq!(h.text("at"), "1.3");
}

#[cfg(feature = "gltf")]
#[test]
fn a_clip_it_has_not_says_which_it_has() {
    let mut h = beside_a_model(
        "no-clip",
        r#"return ui.scene3d { w = 100, h = 100, ui.model { src = "square.glb", animation = "walk" } }"#,
    );
    assert!(
        h.wait_until(|h| {
            h.frame();
            h.error().is_some()
        }),
        "no error"
    );
    let error = h.error().unwrap();
    assert!(error.contains("no animation walk: it has slide"), "{error}");
    assert!(
        raised(r#"ui.scene3d { ui.model { src = "a.glb", animation = 0 } }"#).contains("animation")
    );
}

#[test]
fn tone_mapping_and_exposure_are_checked() {
    let mut h = Harness::inline(
        r#"local stops = ui.state("stops", 0)
        return ui.scene3d { id = "scene", w = 100, h = 100, tone_mapping = "aces", exposure = stops,
            ui.mesh { geometry = "box" } }"#,
    );
    assert_eq!(h.error(), None);
    h.frame();
    assert!(frame_id(&mut h, "scene").is_some());
    assert!(raised(r#"ui.scene3d { tone_mapping = "reinhard" }"#).contains("tone_mapping"));
    assert!(raised(r#"ui.scene3d { antialias = 4 }"#).contains("antialias"));
    let mut h = Harness::inline(
        r#"return ui.scene3d { id = "scene", w = 100, h = 100, antialias = false, ui.mesh { geometry = "box" } }"#,
    );
    assert_eq!(h.error(), None);
    h.frame();
    assert!(frame_id(&mut h, "scene").is_some());
}

#[test]
fn an_orbit_turns_by_keyboard_focused_and_a_flick_coasts_to_a_stop() {
    use zgx::input::{InputEvent, Key, Modifiers};
    let mut h = Harness::inline(PICKED);
    h.frames(3);
    let still = frame_id(&mut h, "scene");
    // A click on nothing focuses it and moves nothing.
    press(&mut h, "scene", (5., 5.), 0.);
    h.frames(3);
    assert_eq!(frame_id(&mut h, "scene"), still, "a click turns nothing");
    h.ui.dispatch(InputEvent::KeyDown {
        key: Key::ArrowRight,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    h.frames(2);
    let turned = frame_id(&mut h, "scene");
    assert_ne!(turned, still, "the arrow turned it");
    // A flick: frames after it's let go, from Rust, then none (the press
    // itself hovers the box: its handler runs then, not as it coasts).
    press(&mut h, "scene", (100., 100.), 40.);
    let calls = h.calls();
    let mut frames = std::collections::HashSet::new();
    for _ in 0..10 {
        h.frame();
        frames.insert(frame_id(&mut h, "scene"));
    }
    assert!(frames.len() > 5, "coasting: {} frames", frames.len());
    assert_eq!(h.calls(), calls, "no script ran as it coasted");
    h.frames(300);
    let settled = frame_id(&mut h, "scene");
    h.frames(10);
    assert_eq!(frame_id(&mut h, "scene"), settled, "still once stopped");
}

/// This thread's frames drawn whole, as a video's are, while it lives.
#[cfg(feature = "gltf")]
struct WholeFrames(bool);
#[cfg(feature = "gltf")]
impl WholeFrames {
    fn on() -> Self {
        Self(zgx::image::set_whole_frames_on_this_thread(true))
    }
}
#[cfg(feature = "gltf")]
impl Drop for WholeFrames {
    fn drop(&mut self) {
        zgx::image::set_whole_frames_on_this_thread(self.0);
    }
}

/// A model that hears a click at its middle, saying so in `clicked`.
#[cfg(feature = "gltf")]
const CLICKED_MODEL: &str = r#"
local clicked = ui.state("clicked", "none")
return ui.column {
    ui.text { id = "clicked", "{clicked}" },
    ui.scene3d {
        id = "scene", w = 200, h = 200,
        camera = { position = { 0, 0, 3 } },
        ui.model { src = "square.glb", fit = 2, on_click = function() clicked:set("clicked") end },
    },
}
"#;

#[cfg(feature = "gltf")]
#[test]
fn a_whole_frame_has_its_model_from_its_first_frame() {
    let _whole = WholeFrames::on();
    let mut h = beside_a_model("whole", CLICKED_MODEL);
    assert_eq!(h.error(), None);
    // No waiting: a video's frame doesn't wait for another thread.
    h.frame();
    press(&mut h, "scene", (100., 100.), 0.);
    assert_eq!(h.text("clicked"), "clicked", "{}", h.tree());
}

/// Its buffer a named pipe, which a read of it waits on until the test writes
/// there: read on the main thread, making the window would wait (until the
/// writer gives up waiting for the test, and then it would show at once).
#[cfg(all(feature = "gltf", unix))]
#[test]
fn a_window_reads_its_model_off_its_main_thread() {
    unsafe extern "C" {
        fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32;
    }
    // Under macOS's temporary directory, a link: found through it all the same.
    let dir = std::env::temp_dir().join(format!("zgx-model-pipe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pipe = dir.join("buffer.bin");
    let name = std::ffi::CString::new(pipe.to_str().unwrap()).unwrap();
    // SAFETY: A path that lives through the call.
    assert_eq!(unsafe { mkfifo(name.as_ptr(), 0o600) }, 0);
    std::fs::write(
        dir.join("square.gltf"),
        r#"{"asset":{"version":"2.0"},"buffers":[{"uri":"buffer.bin","byteLength":60}],
        "bufferViews":[{"buffer":0,"byteLength":48},{"buffer":0,"byteOffset":48,"byteLength":12}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3"},
            {"bufferView":1,"componentType":5123,"count":6,"type":"SCALAR"}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
        "nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}]}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("app.luau"),
        CLICKED_MODEL.replace("square.glb", "square.gltf"),
    )
    .unwrap();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let writer = std::thread::spawn(move || {
        let _ = released.recv_timeout(std::time::Duration::from_secs(5));
        let positions: [f32; 12] = [-0.5, -0.5, 0., 0.5, -0.5, 0., 0.5, 0.5, 0., -0.5, 0.5, 0.];
        let mut bin: Vec<u8> = positions.iter().flat_map(|v| v.to_le_bytes()).collect();
        bin.extend([0_u16, 1, 2, 0, 2, 3].iter().flat_map(|i| i.to_le_bytes()));
        std::fs::write(pipe, bin).unwrap();
    });
    let mut h = Harness::new(|runtime| zgx_script::ScriptHost::new(runtime, dir.join("app.luau")));
    assert_eq!(h.error(), None);
    // The read is waiting on the pipe: a window goes on without its model.
    h.frame();
    press(&mut h, "scene", (100., 100.), 0.);
    assert_eq!(h.text("clicked"), "none", "read on the main thread");
    release.send(()).unwrap();
    let heard = h.wait_until(|h| {
        h.frame();
        press(h, "scene", (100., 100.), 0.);
        h.text("clicked") != "none"
    });
    assert!(heard, "never read: {} {:?}", h.tree(), h.error());
    writer.join().unwrap();
}

#[cfg(feature = "gltf")]
#[test]
fn a_file_that_is_no_model_says_so_whichever_way_it_is_read() {
    for whole in [true, false] {
        let _whole = whole.then(WholeFrames::on);
        let dir =
            std::env::temp_dir().join(format!("zgx-model-bad-{whole}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bad.glb"), b"not a model").unwrap();
        std::fs::write(
            dir.join("app.luau"),
            r#"return ui.scene3d { w = 100, h = 100, ui.model { src = "bad.glb" } }"#,
        )
        .unwrap();
        let mut h =
            Harness::new(|runtime| zgx_script::ScriptHost::new(runtime, dir.join("app.luau")));
        if whole {
            // Told by the time it's mounted, with nothing to wait for.
            assert!(h.error().is_some(), "said nothing: {}", h.tree());
        } else {
            assert!(
                h.wait_until(|h| {
                    h.frame();
                    h.error().is_some()
                }),
                "said nothing"
            );
        }
    }
}

/// A model sliding a unit a second (`animation = "slide"`) posed at `time`,
/// the state `t` starting at T, counting the clicks that land on it. The
/// camera 6 back sees x 1 at 40 points right of the middle.
#[cfg(feature = "gltf")]
const POSED_MODEL: &str = r#"
local t = ui.state("t", T)
local hits = ui.state("hits", 0)
return ui.column {
    ui.text { id = "hits", "{hits}" },
    ui.button { id = "on", w = 10, h = 10, on_click = function() t:set(t:get() + 0.5) end },
    ui.scene3d {
        id = "scene", w = 200, h = 200,
        camera = { position = { 0, 0, 6 } },
        ui.model {
            src = "square.glb",
            animation = "slide",
            time = t,
            on_click = function() hits:set(hits:get() + 1) end,
        },
    },
}
"#;

/// Whether a click at `x` in the scene lands on the model.
#[cfg(feature = "gltf")]
fn lands(h: &mut zgx_script::testing::TestApp, x: f32) -> bool {
    let px = 100. + x / (6. * 22.5_f32.to_radians().tan()) * 100.;
    let before = h.text("hits");
    press(h, "scene", (px, 100.), 0.);
    h.text("hits") != before
}

#[cfg(feature = "gltf")]
#[test]
fn time_poses_the_clip_follows_its_state_and_does_not_play() {
    // Both ways of reading: a frame drawn whole, and a window's off its thread.
    for whole in [true, false] {
        let _whole = whole.then(WholeFrames::on);
        let mut h = beside_a_model(&format!("time-{whole}"), &POSED_MODEL.replace("T)", "1)"));
        assert_eq!(h.error(), None);
        // At 1 s it is a unit along (the square 0.5 either side of x 1)...
        assert!(
            h.wait_until(|h| {
                h.frame();
                lands(h, 1.3)
            }),
            "not at its time: {}",
            h.tree()
        );
        assert!(!lands(&mut h, 0.3), "still where it starts");
        // ...and stays there however many frames go by: nothing plays it.
        h.frames(120);
        assert!(lands(&mut h, 1.2), "it played on");
        assert!(!lands(&mut h, 2.2), "it played on");
        // Half a second more on the state: half a unit along.
        h.click("on");
        h.frame();
        assert!(lands(&mut h, 1.8), "didn't follow its state");
        assert!(!lands(&mut h, 0.8), "didn't follow its state");
    }
}

#[cfg(feature = "gltf")]
#[test]
fn the_same_time_gives_the_same_pose_however_it_got_there() {
    // A chunk of a render starting at 1.5 s, and one that came there through
    // 0.5 s and 1 s: where each edge lies, to a hair either side of it.
    let edges = |h: &mut Harness| [1.9, 1.95, 2.05, 2.1].map(|x| lands(h, x));
    let _whole = WholeFrames::on();
    let mut direct = beside_a_model("same-direct", &POSED_MODEL.replace("T)", "1.5)"));
    direct.frame();
    let mut stepped = beside_a_model("same-stepped", &POSED_MODEL.replace("T)", "0.5)"));
    stepped.frame();
    stepped.click("on");
    stepped.frame();
    stepped.click("on");
    stepped.frame();
    assert_eq!(edges(&mut direct), edges(&mut stepped));
    assert_eq!(edges(&mut direct), [true, true, false, false]);
}

#[cfg(feature = "gltf")]
#[test]
fn time_without_an_animation_says_so() {
    let error = raised(r#"ui.scene3d { ui.model { src = "a.glb", time = 1 } }"#);
    assert!(
        error.contains("`time`") && error.contains("animation"),
        "{error}"
    );
}

/// A square of two triangles facing the camera at z = 0.5, its own
/// geometry (`FACES` its per-triangle form), clicked as a box is.
const OWN: &str = r##"
local clicked = ui.state("clicked", "none")
local square = {
    positions = { -0.5, -0.5, 0.5,  0.5, -0.5, 0.5,  0.5, 0.5, 0.5,  -0.5, 0.5, 0.5 },
    indices = { 1, 2, 3,  1, 3, 4 },
    FACES
}
return ui.column {
    ui.text { id = "clicked", "{clicked}" },
    ui.scene3d {
        id = "scene",
        w = 200,
        h = 200,
        orbit = true,
        camera = { position = { 0, 0, 4 }, fov = 40 },
        ui.mesh {
            geometry = square,
            material = { double_sided = true },
            on_click = function(hit) clicked:set(string.format("%.2f", hit.z)) end,
        },
    },
}
"##;

#[test]
fn a_mesh_of_its_own_triangles_shows_and_is_clicked() {
    for faces in [
        "",
        "colors = { 1, 0, 0,  0, 1, 0,  0, 0, 1,  1, 1, 1 }",
        "face_normals = { 0, 0, 1,  0, 0, 1 }, face_colors = { 1, 0, 0,  0, 0, 1 }",
        "flat = true",
        "flat = true, colors = { 1, 0, 0,  0, 1, 0,  0, 0, 1,  1, 1, 1 }",
    ] {
        let mut h = Harness::inline(&OWN.replace("FACES", faces));
        assert_eq!(h.error(), None, "{faces}");
        h.frame();
        assert!(frame_id(&mut h, "scene").is_some(), "{faces}");
        press(&mut h, "scene", (100., 100.), 0.);
        assert_eq!(h.text("clicked"), "0.50", "{faces}");
    }
}

#[test]
fn a_mesh_of_its_own_triangles_is_checked() {
    let made = |g: &str| raised(&format!("ui.mesh {{ geometry = {g} }}"));
    assert!(made("{}").contains("positions"));
    assert!(made("{ positions = { 0, 0 } }").contains("three a vertex"));
    assert!(made(r#"{ positions = { 0, 0, "z" } }"#).contains("`positions`[3]"));
    assert!(made("{ positions = { 0, 0, 0 }, indices = { 1, 1, 2 } }").contains("from 1 to 1"));
    assert!(made("{ positions = { 0, 0, 0 }, indices = { 0, 1, 1 } }").contains("from 1 to 1"));
    assert!(
        made("{ positions = { 0,0,0, 1,0,0, 0,1,0 }, indices = { 1, 2 } }")
            .contains("three a triangle")
    );
    assert!(
        made("{ positions = { 0,0,0, 1,0,0, 0,1,0 }, colors = { 1, 0 } }")
            .contains("three or four")
    );
    assert!(
        made("{ positions = { 0,0,0, 1,0,0, 0,1,0 }, face_normals = { 0, 0 } }")
            .contains("face_normals")
    );
    assert!(made("true").contains("geometry"));
    // Numbers the renderer can't use: not a number, or too large for 32 bits.
    let triangle = "0,0,0, 1,0,0, 0,1,0";
    for bad in ["0/0", "math.huge", "-math.huge", "1e300"] {
        let positions = format!("{{ positions = {{ {bad},0,0, 1,0,0, 0,1,0 }} }}");
        assert!(made(&positions).contains("`positions`[1]"), "{bad}");
        let normals =
            format!("{{ positions = {{ {triangle} }}, normals = {{ {bad},0,1, 0,0,1, 0,0,1 }} }}");
        assert!(made(&normals).contains("`normals`[1]"), "{bad}");
        let colors =
            format!("{{ positions = {{ {triangle} }}, colors = {{ {bad},0,1, 0,0,1, 0,0,1 }} }}");
        assert!(made(&colors).contains("`colors`[1]"), "{bad}");
        let index = format!("{{ positions = {{ {triangle} }}, indices = {{ {bad},1,2 }} }}");
        assert!(made(&index).contains("`indices`[1]"), "{bad}");
    }
    // Large but finite: its normal overflows to no direction, not to NaN.
    assert_eq!(
        made("{ positions = { 1e30,0,0, 0,1e30,0, 0,0,1e30 } }"),
        "no error"
    );
    // A flat mesh's normals are its faces'; a corner has one colour.
    assert!(
        made(&format!(
            "{{ positions = {{ {triangle} }}, flat = true, normals = {{ 0,0,1, 0,0,1, 0,0,1 }} }}"
        ))
        .contains("`normals`")
    );
    assert!(
        made(&format!(
            "{{ positions = {{ {triangle} }}, colors = {{ 1,0,0, 1,0,0, 1,0,0 }}, face_colors = {{ 1,0,0 }} }}"
        ))
        .contains("not both")
    );
    // Booleans are booleans: a state wouldn't be followed.
    assert!(
        made(&format!(
            "{{ positions = {{ {triangle} }}, flat = \"yes\" }}"
        ))
        .contains("`flat` is a boolean")
    );
    assert!(
        raised("ui.mesh { material = { double_sided = ui.state(\"on\", true) } }")
            .contains("`material.double_sided` is a boolean")
    );
    assert_eq!(made("{ positions = {} }"), "no error");
    assert_eq!(made("{ positions = { 0,0,0, 1,0,0, 0,1,0 } }"), "no error");
}

/// A plugin granted nothing shows a scene of its own triangles, and hears
/// clicks on them: `ui.scene3d` asks no capability.
#[test]
fn a_plugin_granted_nothing_shows_and_picks_its_own_mesh() {
    use zgx_script::{Plugin, ScriptHost};
    let dir = std::env::temp_dir().join(format!("zgx-scene3d-plugin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    let plugin = OWN
        .replace("FACES", "face_colors = { 1, 0, 0,  0, 0, 1 }")
        .replace(
            "return ui.column {",
            "ui.contribute { target = \"#slot\", after = ui.column {",
        )
        .trim_end()
        .to_owned()
        + " }\n";
    std::fs::write(&entry, plugin).unwrap();
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"return ui.column { ui.text { id = "slot", "app" } }"#,
        );
        host.set_plugins(vec![Plugin {
            name: "cad".into(),
            entry,
            grants: vec![],
        }]);
        host
    });
    h.frames(2);
    assert_eq!(h.error(), None);
    assert!(frame_id(&mut h, "scene").is_some(), "{}", h.tree());
    press(&mut h, "scene", (100., 100.), 0.);
    assert_eq!(h.text("clicked"), "0.50");
}

/// A mesh with no triangles (a boolean of two solids that don't meet) is a
/// mesh that shows nothing, and clicks pass it.
#[test]
fn a_mesh_of_no_triangles_shows_nothing() {
    let mut h = Harness::inline(
        &OWN.replace("positions = {", "positions = {} or {")
            .replace("indices = { 1, 2, 3,  1, 3, 4 },", "")
            .replace("FACES", ""),
    );
    assert_eq!(h.error(), None);
    h.frame();
    assert!(frame_id(&mut h, "scene").is_some());
    press(&mut h, "scene", (100., 100.), 0.);
    assert_eq!(h.text("clicked"), "none");
}

/// What a plugin's meshes hold counts against its memory, though it lives
/// outside its VM; what it lets go of is room again.
#[test]
fn a_plugin_meshes_count_against_its_memory() {
    use zgx_script::{Limits, Plugin, ScriptHost};
    let dir = std::env::temp_dir().join(format!("zgx-scene3d-budget-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(
        &entry,
        r##"
local result = ui.state("result", "")
ui.contribute { target = "#slot", after = ui.text { id = "got", "{result}" } }
local positions = table.create(3 * 3000, 0)
local function mesh() return ui.mesh { geometry = { positions = positions } } end
-- Each holds some 108 KB, nothing of it in the VM: 200 of them are far more
-- than 4 MB holds, if they were held.
local thrown = 0
for i = 1, 200 do
    local ok = pcall(mesh)
    if not ok then break end
    thrown = i
end
local kept, held, why = {}, 0, ""
for i = 1, 200 do
    local ok, made = pcall(mesh)
    if not ok then why = tostring(made) break end
    kept[i] = made
    held = i
end
local short = string.find(why, "needs 0.1 MB: a plugin's memory is 4 MB", 1, true) ~= nil
kept = {}
local again = pcall(mesh)
result:set(`{thrown} {held} {short} {again}`)
"##,
    )
    .unwrap();
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"return ui.column { ui.text { id = "slot", "app" } }"#,
        )
        .limits(Limits {
            plugin_memory: 4 << 20,
            ..Default::default()
        });
        host.set_plugins(vec![Plugin {
            name: "cad".into(),
            entry,
            grants: vec![],
        }]);
        host
    });
    h.frames(2);
    let got = h.text("got");
    let parts: Vec<&str> = got.split(' ').collect();
    // Thrown away, each is collected to make room for the next.
    assert_eq!(parts[0], "200", "{got}");
    let held: usize = parts[1].parse().unwrap();
    assert!((1..50).contains(&held), "{got}");
    assert_eq!(parts[2], "true", "{got}");
    assert_eq!(parts[3], "true", "{got}");
}
