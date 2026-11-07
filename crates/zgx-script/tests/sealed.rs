//! A sealed region (`ui.sealed`, a Settings page): no plugin contributes into
//! it, replaces it, filters it, targets it or draws over it, but for each
//! plugin's own page (`ui.plugin_page`), which only that plugin fills and
//! which it can't get out of.
mod common;
use common::Harness;
use std::path::PathBuf;
use zgx_script::{Capability, Plugin, ScriptHost};

fn entry(name: &str, code: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "zgx-sealed-{}-{:?}-{name}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(&entry, code).unwrap();
    entry
}
fn with(plugins: &[(&'static str, &str)]) -> Harness {
    let plugins: Vec<Plugin> = plugins
        .iter()
        .map(|(name, code)| Plugin {
            name: (*name).into(),
            entry: entry(name, code),
            grants: vec![Capability::Override("*".into())],
        })
        .collect();
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.set_plugins(plugins);
        host
    })
}
/// Whether the pointer at `x`, `y` finds `id`.
fn hits(h: &mut Harness, x: f32, y: f32, id: &str) -> bool {
    let at = h.find(id).unwrap();
    let scene = h.ui.scene.borrow();
    h.ui.input.pointer_target(&scene, x, y) == Some(at)
}
const APP: &str = r##"
return ui.column { w_full = true, h_full = true,
    ui.text { id = "after", "After" },
    ui.sealed { id = "settings", w = 300, h = 200,
        ui.column { w_full = true, h_full = true,
            ui.button { id = "app-setting", w = 300, h = 40, "Setting" },
            ui.plugin_page { plugin = "mine", w = 300, h = 100 },
            ui.button { id = "app-below", w = 300, h = 40, "Below" },
        },
    },
}
"##;

#[test]
fn no_plugin_reaches_into_it() {
    let mut h = with(&[(
        "other",
        r##"
ui.contribute { target = "#app-setting", replace = ui.text { id = "replaced", "x" } }
ui.contribute { target = "#app-setting", before = ui.text { id = "added", "x" } }
ui.filter { target = "#app-setting", text = function() return "Sure" end }
"##,
    )]);
    h.frames(4);
    assert!(h.find("app-setting").is_some());
    assert!(h.find("replaced").is_none());
    assert!(h.find("added").is_none());
    assert_eq!(h.text("app-setting"), "Setting");
}

#[test]
fn nothing_a_plugin_opens_is_over_it() {
    let mut h = with(&[(
        "other",
        r##"
local open = ui.state("open", true)
ui.contribute { target = "#after", after = ui.modal { open = open, z = 2147483647,
    ui.div { id = "plugin-modal", w = 800, h = 800, z = 2147483647, on_click = function() end } } }
"##,
    )]);
    h.frames(5);
    assert!(h.find("plugin-modal").is_some());
    let b = h.bounds("app-setting");
    assert!(hits(&mut h, b.x + 5., b.y + 5., "app-setting"));
    h.click("app-setting");
    assert_eq!(h.error(), None);
}

#[test]
fn a_page_shows_its_plugins_alone() {
    let mut h = with(&[
        (
            "mine",
            r##"ui.contribute("plugin_page", ui.button { id = "my-page", w = 100, h = 30, "Mine" })"##,
        ),
        (
            "other",
            r##"ui.contribute("plugin_page", ui.button { id = "other-page", w = 100, h = 30, "Other" })"##,
        ),
    ]);
    h.frames(4);
    assert!(h.find("my-page").is_some());
    assert!(h.find("other-page").is_none(), "its page is not this one");
    let b = h.bounds("my-page");
    assert!(hits(&mut h, b.x + 5., b.y + 5., "my-page"));
}

#[test]
fn what_a_page_opens_stays_in_its_box() {
    let mut h = with(&[(
        "mine",
        r##"
local open = ui.state("open", true)
ui.contribute("plugin_page", ui.column {
    ui.div { id = "too-big", absolute = true, left = -500, top = -500, w = 3000, h = 3000,
        z = 2147483647, on_click = function() end },
    ui.modal { open = open, z = 2147483647,
        ui.div { id = "page-modal", w = 3000, h = 3000, on_click = function() end } },
})
"##,
    )]);
    h.frames(5);
    assert!(h.find("too-big").is_some());
    assert!(h.find("page-modal").is_some());
    let (above, below) = (h.bounds("app-setting"), h.bounds("app-below"));
    assert!(hits(&mut h, above.x + 5., above.y + 5., "app-setting"));
    assert!(hits(&mut h, below.x + 5., below.y + 5., "app-below"));
}

#[test]
fn only_the_app_makes_them() {
    for code in [
        r##"ui.contribute { target = "#after", after = ui.sealed { ui.text { "x" } } }"##,
        r##"ui.contribute { target = "#after", after = ui.plugin_page { plugin = "other" } }"##,
    ] {
        let h = with(&[("other", code)]);
        let error = h.error().unwrap_or_default();
        assert!(
            error.contains("only the app's own script"),
            "{code}: {error}"
        );
    }
}

#[test]
fn a_page_outside_a_sealed_region_shows_nothing_and_says_so() {
    let h = Harness::new(|runtime| {
        ScriptHost::inline(
            runtime,
            "app",
            r##"return ui.column { id = "c", ui.plugin_page { plugin = "mine" } }"##,
        )
    });
    h.frames(2);
    let error = h.error().unwrap_or_default();
    assert!(error.contains("ui.sealed region"), "{error}");
}

const BESIDE_THE_PAGE: &str = r##"
return ui.column { w_full = true, h_full = true,
    ui.sealed { id = "settings", w = 300, h = 200,
        ui.row { w_full = true, h_full = true,
            ui.plugin_page { plugin = "mine", w = 100, h = 100 },
            ui.div { id = "blank", w = 200, h = 200 },
        },
    },
}
"##;

/// The page's box is what cuts it, on every side: past its right and bottom,
/// where nothing painted after it covers it, it neither shows nor takes the
/// pointer, and a modal it opens is cut the same.
#[test]
fn a_page_is_cut_to_its_box_on_every_side() {
    let code = r##"
local open = ui.state("open", true)
ui.contribute("plugin_page", ui.column {
    ui.div { id = "too-big", absolute = true, left = 0, top = 0, w = 3000, h = 3000,
        on_click = function() end },
    ui.modal { open = open,
        ui.div { id = "page-modal", w = 3000, h = 3000, on_click = function() end } },
})
"##;
    let entry = entry("cut", code);
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", BESIDE_THE_PAGE);
        host.set_plugins(vec![Plugin {
            name: "mine".into(),
            entry,
            grants: Vec::new(),
        }]);
        host
    });
    h.frames(5);
    let (page, modal) = (h.find("too-big").unwrap(), h.find("page-modal").unwrap());
    for (id, node) in [("too-big", page), ("page-modal", modal)] {
        let shown = h.ui.scene.borrow().visible_bounds(node).expect(id);
        assert!(
            shown.width <= 100. && shown.height <= 100.,
            "{id} shows as {shown:?}"
        );
    }
    // Past the page's right and its bottom: nothing of the plugin's takes it.
    for (x, y) in [(150., 50.), (50., 150.), (250., 150.)] {
        let scene = h.ui.scene.borrow();
        let target = h.ui.input.pointer_target(&scene, x, y);
        assert!(
            target.is_none_or(|t| !scene.is_within(t, page) && !scene.is_within(t, modal)),
            "at {x},{y}: {target:?}"
        );
    }
}

/// Red where it reads anything behind it, blue where it reads nothing; and
/// magenta everywhere past its box, as far as it's allowed to reach.
const READER: &str = r#"
local shader = ui.layer_shader([[
fn layer(input: LayerInput) -> vec4<f32> {
    if any(input.p < vec2(0.0)) || any(input.p >= input.size) {
        return vec4(1.0, 0.0, 1.0, 1.0);
    }
    let behind = zgx_backdrop(input.p);
    return vec4(step(0.5, behind.a), 0.0, 1.0 - step(0.5, behind.a), 1.0);
}]])
"#;
fn over(plugin: &str, app: &str) -> Harness {
    let (plugin, app) = (plugin.to_owned(), app.to_owned());
    let plugins = vec![Plugin {
        name: "mine".into(),
        entry: entry("mine", &plugin),
        grants: vec![Capability::Override("*".into())],
    }];
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", &app);
        host.set_plugins(plugins);
        host
    })
}
fn pixels(h: &mut Harness) -> (Vec<u8>, u32) {
    h.ui.prepare_frame();
    let scene = h.ui.scene.borrow();
    let size = 600u32;
    let mut gpu = zgx_gpu::GpuRenderer::new(size, size).unwrap();
    let frame = zgx::scene::Rect::new(0., 0., size as f32, size as f32);
    gpu.render(&scene, &[frame]).unwrap();
    (gpu.readback().unwrap(), size)
}
fn at(pixels: &(Vec<u8>, u32), x: f32, y: f32) -> [u8; 4] {
    let i = (y as u32 * pixels.1 + x as u32) as usize * 4;
    pixels.0[i..i + 4].try_into().unwrap()
}
fn centre(h: &mut Harness, id: &str) -> (f32, f32) {
    let node = h.find(id).unwrap();
    let scene = h.ui.scene.borrow();
    let bounds = scene.visible_bounds(node).unwrap();
    (
        bounds.x + bounds.width * 0.5,
        bounds.y + bounds.height * 0.5,
    )
}

/// A plugin's page reads its own group alone: nothing of the sealed region's
/// under it, and it can't paint past its box.
#[test]
fn a_plugins_shader_in_its_page_reads_nothing_of_the_region_around_it() {
    let plugin = format!(
        r##"{READER}
ui.contribute("plugin_page", ui.div {{ id = "reader", w = 100, h = 30,
    layer_shader = shader, layer_bleed = 400, layer_reach = 400 }})"##
    );
    let mut h = over(
        &plugin,
        r##"return ui.column { w_full = true, h_full = true, bg = "#00ff00",
            ui.sealed { id = "settings", w = 300, h = 200, bg = "#00ff00",
                ui.column { w_full = true, h_full = true,
                    ui.plugin_page { plugin = "mine", w = 300, h = 100 },
                    ui.div { id = "below", w = 300, h = 60 },
                },
            },
        }"##,
    );
    h.frames(3);
    assert_eq!(h.host.error().get(), None);
    let (x, y) = centre(&mut h, "reader");
    let (bx, by) = centre(&mut h, "below");
    let shown = pixels(&mut h);
    assert_eq!(
        at(&shown, x, y),
        [0, 0, 255, 255],
        "it read the region under its page"
    );
    assert_eq!(
        at(&shown, bx, by),
        [0, 255, 0, 255],
        "it painted past its page"
    );
}

/// What's over everything is painted after a plugin: its shader can't paint
/// over it, however far its bleed goes.
#[test]
fn a_plugins_shader_can_not_paint_over_a_decision() {
    let plugin = format!(
        r##"{READER}
ui.contribute {{ target = "#slot", after = ui.div {{ id = "reader", w = 100, h = 30,
    layer_shader = shader, layer_bleed = 500, layer_reach = 500 }} }}"##
    );
    let mut h = over(
        &plugin,
        r##"return ui.column { w_full = true, h_full = true,
            ui.div { id = "slot", w = 200, h = 40 },
            ui.decision { key = "allow", w = 120, h = 40,
                ui.div { id = "allow", w = 120, h = 40, bg = "#00ff00" } },
        }"##,
    );
    h.frames(3);
    assert_eq!(h.host.error().get(), None);
    assert!(h.find("reader").is_some(), "the plugin's in the window");
    let (x, y) = centre(&mut h, "allow");
    let shown = pixels(&mut h);
    assert_eq!(
        at(&shown, x, y),
        [0, 255, 0, 255],
        "a plugin painted over a decision"
    );
}

/// That a shader reading nothing above isn't for having no way to read: over the
/// app's own content, where it may, a plugin's does read it.
#[test]
fn outside_a_sealed_region_a_plugins_shader_reads_what_is_behind_it() {
    let plugin = format!(
        r##"{READER}
ui.contribute {{ target = "#slot", replace = ui.div {{ id = "reader", w = 100, h = 30,
    layer_shader = shader }} }}"##
    );
    let mut h = over(
        &plugin,
        r##"return ui.column { w_full = true, h_full = true, bg = "#00ff00",
            ui.div { id = "slot", w = 200, h = 40 },
        }"##,
    );
    h.frames(3);
    assert_eq!(h.host.error().get(), None);
    let (x, y) = centre(&mut h, "reader");
    let shown = pixels(&mut h);
    assert_eq!(at(&shown, x, y), [255, 0, 0, 255]);
}
