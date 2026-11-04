//! A decision control (`ui.decision`): no plugin reaches it, and it is drawn
//! and hit over whatever a plugin opens, holding its room in flow and
//! following its place. What a plugin would do to remove it is refused, what
//! a builder hides from that is reported (`ScriptHost::decision_shown`), and
//! a plugin's popups stay in the window, under it, while one is up.
mod common;
use common::Harness;
use std::{cell::Cell, path::PathBuf};
use zgx::input::{InputEvent, PointerButton};
use zgx_script::{Capability, Plugin, ScriptHost};

thread_local! { static DIRS: Cell<u32> = const { Cell::new(0) }; }
fn plugin(name: &str, code: &str) -> PathBuf {
    let at = DIRS.with(|dirs| dirs.replace(dirs.get() + 1));
    let dir = std::env::temp_dir().join(format!(
        "zgx-decision-{}-{:?}-{at}-{name}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(&entry, code).unwrap();
    entry
}
fn with(app: &'static str, code: &str, grants: Vec<Capability>) -> Harness {
    let entry = plugin("p", code);
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", app);
        host.set_plugins(vec![Plugin {
            name: "p".into(),
            entry,
            grants,
        }]);
        host
    })
}
/// Whether the pointer at `x`, `y` finds `id`.
fn hits(h: &mut Harness, x: f32, y: f32, id: &str) -> bool {
    let at = h.find(id).unwrap();
    let scene = h.ui.scene.borrow();
    h.ui.input.pointer_target(&scene, x, y) == Some(at)
}
fn middle(h: &mut Harness, id: &str) -> (f32, f32) {
    let b = h.bounds(id);
    (b.x + b.width / 2., b.y + b.height / 2.)
}

const APP: &str = r##"
local allowed = ui.state("allowed", 0)
return ui.column { id = "page", w_full = true, h_full = true,
    ui.text { id = "before", "Before" },
    ui.decision { key = "approve", id = "held",
        ui.button { id = "allow", w = 80, h = 30, "Allow",
            on_click = function() allowed:set(allowed:peek() + 1) end },
    },
    ui.text { id = "after", "After" },
    ui.text { id = "allowed", "{allowed}" },
}
"##;

#[test]
fn it_holds_its_room_in_flow_and_lies_over_it() {
    let mut h = Harness::new(|runtime| ScriptHost::inline(runtime, "app", APP));
    h.frames(4);
    let (before, allow, after) = (h.bounds("before"), h.bounds("allow"), h.bounds("after"));
    assert_eq!(allow.y, before.y + before.height, "where its room is");
    assert_eq!(after.y, allow.y + allow.height, "what follows is after it");
    assert_eq!((allow.width, allow.height), (80., 30.));
    h.click("allow");
    h.frames(2);
    assert_eq!(h.text("allowed"), "1");
    assert!(h.host.decision_shown("approve"));
    assert!(!h.host.decision_shown("never"));
}

const COVERS: &str = r##"
local open = ui.state("open", true)
ui.contribute { target = "#after", after = ui.modal { open = open, z = 2147483647,
    ui.div { id = "plugin-modal", w = 600, h = 400, z = 2147483647, on_click = function() end } } }
ui.contribute { target = "#after", after = ui.layer { open = open, z = 2147483647,
    ui.div { id = "plugin-layer", w = 600, h = 400, z = 2147483647, on_click = function() end } } }
"##;

#[test]
fn it_is_above_a_plugins_modal_and_layer_and_live_under_them() {
    let mut h = with(APP, COVERS, Vec::new());
    h.frames(5);
    assert!(h.find("plugin-modal").is_some());
    assert!(h.find("plugin-layer").is_some());
    let (x, y) = middle(&mut h, "allow");
    assert!(hits(&mut h, x, y, "allow"));
    let (allow, modal) = (h.find("allow").unwrap(), h.find("plugin-modal").unwrap());
    let order: Vec<_> = (h.ui.scene.borrow().layer_items(None).iter())
        .map(|item| item.id)
        .collect();
    let at = |id| order.iter().position(|item| *item == id);
    assert!(at(allow) > at(modal), "drawn after it");
    h.click("allow");
    h.frames(2);
    assert_eq!(h.text("allowed"), "1");
}

#[test]
fn a_plugin_can_not_make_one() {
    let h = with(
        APP,
        r##"ui.contribute { target = "#after", after = ui.decision { key = "fake", ui.text { "Fake" } } }"##,
        Vec::new(),
    );
    h.frames(3);
    assert!(!h.host.decision_shown("fake"));
    let error = h.error().unwrap_or_default();
    assert!(error.contains("only the app's own script"), "{error}");
}

#[test]
fn no_plugin_reaches_what_it_holds() {
    let mut h = with(
        r##"
return ui.column {
    ui.decision { key = "k",
        ui.slot("inside"),
        ui.each(ui.state("rows", { 1 }), function()
            return ui.button { id = "in-row", w = 40, h = 20, "Row" }
        end),
        ui.button { id = "in", w = 40, h = 20, "In" },
    },
}
"##,
        r##"
ui.contribute { target = "#in", replace = ui.text { id = "replaced", "x" } }
ui.contribute { target = "#in", before = ui.text { id = "added", "x" } }
ui.contribute { target = "#in-row", before = ui.text { id = "added-row", "x" } }
ui.filter { target = "#in", text = function() return "Sure" end }
ui.contribute("inside", ui.text { id = "slotted", "x" })
"##,
        vec![Capability::Override("*".into())],
    );
    h.frames(4);
    assert!(h.find("in").is_some());
    assert!(h.find("in-row").is_some());
    for id in ["replaced", "added", "added-row", "slotted"] {
        assert!(h.find(id).is_none(), "{id} reached it");
    }
}

const NESTED: &str = r##"
return ui.column { w_full = true, h_full = true,
    ui.div { id = "bar",
        ui.show(ui.state("on", true), ui.row { id = "inner",
            ui.decision { key = "k", ui.button { id = "allow", w = 80, h = 30, "Allow" } } }),
    },
}
"##;

#[test]
fn removing_what_holds_one_is_refused_and_reported() {
    for code in [
        r##"ui.contribute { target = "#bar", replace = ui.text { id = "gone", "x" } }"##,
        r##"ui.filter { target = "#bar", hidden = function() return true end }"##,
        r##"ui.filter { target = "#bar", disabled = function() return true end }"##,
    ] {
        let mut h = with(NESTED, code, vec![Capability::Override("*".into())]);
        h.frames(4);
        assert!(h.find("gone").is_none());
        assert!(h.host.decision_shown("k"), "{code}");
        let error = h.error().unwrap_or_default();
        assert!(
            error.contains("holds a decision control"),
            "{code}: {error}"
        );
    }
}

#[test]
fn what_a_builder_hides_from_the_refusal_is_reported() {
    let mut h = with(
        r##"
local Built = ui.component(function()
    return ui.div { id = "built", ui.decision { key = "k",
        ui.button { id = "allow", w = 80, h = 30, "Allow" } } }
end)
return ui.column {
    ui.div { id = "outer", Built {} },
}
"##,
        r##"ui.contribute { target = "#outer", replace = ui.text { id = "gone", "x" } }"##,
        vec![Capability::Override("*".into())],
    );
    h.frames(4);
    assert!(h.find("gone").is_some(), "no refusal sees into a builder");
    assert!(!h.host.decision_shown("k"));
}

const SCROLLED: &str = r##"
return ui.column { w_full = true, h_full = true,
    ui.scroll { id = "s", h = 100, w = 200,
        ui.column {
            ui.div { h = 60 },
            ui.decision { key = "k", ui.button { id = "allow", w = 80, h = 80, "Allow" } },
            ui.div { h = 600 },
        },
    },
    ui.button { id = "under", w = 200, h = 200, "Under" },
}
"##;

#[test]
fn it_scrolls_with_its_place_and_is_cut_to_what_shows() {
    let mut h = Harness::new(|runtime| ScriptHost::inline(runtime, "app", SCROLLED));
    h.frames(4);
    let scroller = h.bounds("s");
    let allow = h.bounds("allow");
    assert_eq!(allow.y, scroller.y + 60.);
    // 40 of its 80 show: cut at the scroller's edge, and past it what is
    // there takes the pointer.
    assert!(h.host.decision_shown("k"));
    let x = allow.x + 10.;
    assert!(hits(&mut h, x, scroller.y + 90., "allow"));
    assert!(hits(&mut h, x, scroller.y + 110., "under"));
    h.scroll("s", 60.);
    h.frames(4);
    assert_eq!(h.bounds("allow").y, scroller.y, "it moved as its place did");
    assert!(hits(&mut h, x, scroller.y + 10., "allow"));
    assert!(h.host.decision_shown("k"));
    h.scroll("s", 200.);
    h.frames(4);
    assert!(!h.host.decision_shown("k"), "scrolled away");
    assert!(!hits(&mut h, x, scroller.y + 10., "allow"));
}

#[test]
fn a_plugins_window_makes_it_not_shown() {
    let mut h = with(
        APP,
        r##"
local open = ui.state("open", true)
ui.window { id = "plugin-window", open = open, ui.text { "Over" } }
"##,
        vec![Capability::Windows],
    );
    h.frames(4);
    assert!(h.find("allow").is_some());
    assert!(!h.host.decision_shown("approve"));
}

const POPOVER_APP: &str = r##"
local up = ui.state("up", false)
ui.command { id = "up", run = function() up:set(true) end }
ui.command { id = "down", run = function() up:set(false) end }
return ui.column { w_full = true, h_full = true,
    ui.popover { open = ui.state("app-open", true), label = "App's",
        anchor = ui.button { id = "app-trigger", w = 50, h = 20, "App" },
        ui.div { id = "app-panel", w = 300, h = 300 } },
    ui.text { id = "after", "After" },
    ui.show(up, ui.decision { key = "k", ui.button { id = "allow", w = 80, h = 30, "Allow" } }),
}
"##;
const DECIDES: &str = r##"
local up = ui.state("up", false)
ui.command { id = "up", run = function() up:set(true) end }
return ui.column { w_full = true, h_full = true,
    ui.text { id = "after", "After" },
    ui.show(up, ui.decision { key = "k", ui.button { id = "allow", w = 80, h = 30, "Allow" } }),
}
"##;
const POPUP: &str = r##"
ui.contribute { target = "#after", after = ui.popover { open = ui.state("open", true), label = "Plugin's",
    anchor = ui.button { id = "trigger", w = 50, h = 20, "Plugin" },
    ui.div { id = "plugin-panel", w = 3000, h = 3000 } } }
"##;

fn popup_of(h: &mut Harness, id: &str) -> bool {
    let at = h.find(id).unwrap();
    let scene = h.ui.scene.borrow();
    scene.popups().iter().any(|root| scene.is_within(at, *root))
}

#[test]
fn a_plugins_popup_stays_in_the_window_while_one_is_up() {
    let mut h = with(POPOVER_APP, POPUP, Vec::new());
    h.ui.set_popup_screens(vec![zgx::scene::Rect::new(-1e5, -1e5, 2e5, 2e5)]);
    h.frames(5);
    assert!(popup_of(&mut h, "plugin-panel"), "free before");
    assert!(popup_of(&mut h, "app-panel"));
    assert!(h.host.run_command("up"));
    h.frames(5);
    assert!(!popup_of(&mut h, "plugin-panel"), "in the window now");
    assert!(popup_of(&mut h, "app-panel"), "the app's stays a window");
    let (x, y) = middle(&mut h, "allow");
    assert!(hits(&mut h, x, y, "allow"), "over the overflowing popup");
    assert!(h.host.run_command("down"));
    h.frames(5);
    assert!(popup_of(&mut h, "plugin-panel"), "free again");
}

/// How many popup windows show.
fn popups(app: &zgx_script::testing::TestApp) -> usize {
    app.ui.scene.borrow().popups().len()
}

#[test]
fn a_plugins_tooltip_stays_in_the_window_while_one_is_up() {
    let mut h = with(
        DECIDES,
        r##"ui.contribute { target = "#after", after = ui.button { id = "tipped", w = 50, h = 20, "Tip",
            tooltip = "A tip that overflows" } }"##,
        Vec::new(),
    );
    h.ui.set_popup_screens(vec![zgx::scene::Rect::new(-1e5, -1e5, 2e5, 2e5)]);
    h.frames(3);
    h.hover("tipped");
    h.wait_until(|h| popups(h) == 1);
    assert!(h.host.run_command("up"));
    h.frames(5);
    assert_eq!(popups(&h), 0, "in the window now");
}

#[test]
fn a_plugins_context_menu_stays_in_the_window_while_one_is_up() {
    let mut h = with(
        DECIDES,
        r##"ui.contribute { target = "#after", after = ui.button { id = "menued", w = 50, h = 20, "Menu",
            context_menu = function() return { ui.menu_item { "One" } } end } }"##,
        Vec::new(),
    );
    h.ui.set_popup_screens(vec![zgx::scene::Rect::new(-1e5, -1e5, 2e5, 2e5)]);
    h.frames(3);
    let b = h.bounds("menued");
    let (x, y) = (b.x + 5., b.y + 5.);
    h.ui.dispatch(InputEvent::PointerDown {
        x,
        y,
        button: PointerButton::Secondary,
    });
    h.ui.dispatch(InputEvent::PointerUp {
        x,
        y,
        button: PointerButton::Secondary,
    });
    h.frames(5);
    assert_eq!(popups(&h), 1, "a window of its own, with no decision up");
    assert!(h.host.run_command("up"));
    h.frames(5);
    assert_eq!(popups(&h), 0, "in the window now");
}

#[test]
fn a_popup_in_a_plugins_slot_contribution_stays_in_the_window_while_one_is_up() {
    let mut h = with(
        r##"
local up = ui.state("up", false)
ui.command { id = "up", run = function() up:set(true) end }
return ui.column { w_full = true, h_full = true,
    ui.slot("bar"),
    ui.show(up, ui.decision { key = "k", ui.button { id = "allow", w = 80, h = 30, "Allow" } }),
}
"##,
        r##"ui.contribute("bar", ui.popover { open = ui.state("open", true), label = "Plugin's",
    anchor = ui.button { id = "trigger", w = 50, h = 20, "Plugin" },
    ui.div { id = "plugin-panel", w = 3000, h = 3000 } })"##,
        Vec::new(),
    );
    h.ui.set_popup_screens(vec![zgx::scene::Rect::new(-1e5, -1e5, 2e5, 2e5)]);
    h.frames(5);
    assert!(popup_of(&mut h, "plugin-panel"));
    assert!(h.host.run_command("up"));
    h.frames(5);
    assert!(!popup_of(&mut h, "plugin-panel"));
}
