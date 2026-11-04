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
