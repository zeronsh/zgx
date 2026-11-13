//! `ui.controls` lists the controls on screen as the accessibility tree has them, popups last, and
//! `ui.press` presses one as `zgx inspect click` does; a stale handle is nothing.
mod common;
use common::Harness;

const APP: &str = r##"
local log = ui.state("log", "none")
local result = ui.state("result", "")
local listing = ui.state("listing", "")
local shown = ui.state("shown", true)
local menu = ui.state("menu", false)
local handles = {}
ui.command { id = "scan", run = function()
    local names = {}
    handles = {}
    for _, c in ui.controls() do
        table.insert(names, c.role .. ":" .. c.name)
        handles[c.name] = c.handle
    end
    listing:set(table.concat(names, "|"))
end }
ui.command { id = "press", run = function() result:set(tostring(ui.press(handles["Save"] or -1))) end }
ui.command { id = "press_menu", run = function() result:set(tostring(ui.press(handles["Copy"] or -1))) end }
ui.command { id = "hide", run = function() shown:set(false) end }
ui.command { id = "bogus", run = function() result:set(tostring(ui.press(1e15)) .. " " .. tostring(ui.press(-5)) .. " " .. tostring(ui.press(0 / 0))) end }
ui.command { id = "limit", run = function() listing:set(tostring(#ui.controls { limit = 2 })) end }
ui.command { id = "first", run = function()
    local c = ui.controls()[1]
    listing:set(string.format("%s %s %s %s %s", c.role, tostring(c.x >= 0), tostring(c.w > 0 and c.h > 0), tostring(c.disabled), type(c.handle)))
end }
return ui.column { gap = 8, w_full = true, h_full = true,
    ui.text { id = "log", function() return log:get() end },
    ui.text { id = "result", function() return result:get() end },
    ui.text { id = "listing", function() return listing:get() end },
    ui.button { id = "chain", "Chain", on_click = function() result:set(tostring(ui.press(handles["Save"] or -1))) end },
    ui.show(shown, ui.button { id = "save", "Save", on_click = function() log:set("saved") end }),
    ui.button { id = "off", "Off", disabled = true },
    ui.div { id = "hidden", hidden = true, ui.button { "Never" } },
    ui.checkbox { label = "Agree", value = ui.state("agree", false) },
    ui.menu { id = "m", open = menu,
        anchor = ui.button { id = "opener", "Menu", on_click = function() menu:set(true) end },
        build = function() return ui.column { ui.menu_item { "Copy", on_click = function() log:set("copied") end } } end },
}
"##;

fn run(h: &mut Harness, command: &str) {
    assert!(h.host.run_command(command), "{command}");
    h.frames(2);
    assert_eq!(h.error(), None);
}

#[test]
fn controls_are_listed_in_order_without_hidden_ones_or_plain_text() {
    let mut h = Harness::inline(APP);
    h.frames(3);
    run(&mut h, "scan");
    let listing = h.text("listing");
    assert_eq!(
        listing, "button:Chain|button:Save|button:Off|checkbox:Agree|button:Menu",
        "{listing}"
    );
}

#[test]
fn an_open_menu_comes_last_with_its_items() {
    let mut h = Harness::inline(APP);
    h.frames(3);
    h.click("opener");
    h.frames(3);
    run(&mut h, "scan");
    let listing = h.text("listing");
    let (menu, copy) = (
        listing.find("menu:").expect("the menu panel"),
        listing.find("menuitem:Copy").expect("its item"),
    );
    assert!(
        listing.starts_with("button:Chain|button:Save|"),
        "{listing}"
    );
    assert!(
        menu > listing.find("button:Menu").unwrap() && copy > menu,
        "{listing}"
    );
}

#[test]
fn a_control_says_where_it_is_and_whether_it_is_disabled() {
    let mut h = Harness::inline(APP);
    h.frames(3);
    run(&mut h, "first");
    assert_eq!(h.text("listing"), "button true true false number");
    run(&mut h, "limit");
    assert_eq!(h.text("listing"), "2");
}

#[test]
fn press_clicks_a_control_and_a_menu_item_runs() {
    let mut h = Harness::inline(APP);
    h.frames(3);
    run(&mut h, "scan");
    run(&mut h, "press");
    assert_eq!(h.text("result"), "true");
    assert_eq!(h.text("log"), "saved");
    h.click("opener");
    h.frames(3);
    run(&mut h, "scan");
    run(&mut h, "press_menu");
    assert_eq!(h.text("result"), "true");
    assert_eq!(h.text("log"), "copied");
}

#[test]
fn a_stale_handle_presses_nothing_and_says_so() {
    let mut h = Harness::inline(APP);
    h.frames(3);
    run(&mut h, "scan");
    // The button goes: its handle is stale, a press is false and does nothing.
    run(&mut h, "hide");
    run(&mut h, "press");
    assert_eq!(h.text("result"), "false");
    assert_eq!(h.text("log"), "none", "nothing ran");
    // Nor does a number that names no node, or none at all.
    run(&mut h, "bogus");
    assert_eq!(h.text("result"), "false false false");
}

/// Input within input would go to what the outer press captured, so a press from a handler refuses: false
/// and nothing runs, with no panic.
#[test]
fn a_press_from_a_handler_is_refused() {
    let mut h = Harness::inline(APP);
    h.frames(3);
    run(&mut h, "scan");
    h.click("chain");
    h.frames(2);
    assert_eq!(h.error(), None);
    assert_eq!(h.text("result"), "false");
    assert_eq!(h.text("log"), "none");
}

const CHECKED: &str = r##"
local listing = ui.state("listing", "")
local open = ui.state("open", true)
local wrap = ui.state("wrap", true)
ui.command { id = "scan", run = function()
    local names = {}
    for _, c in ui.controls() do
        table.insert(names, c.role .. ":" .. c.name .. ":" .. tostring(c.checked))
    end
    listing:set(table.concat(names, "|"))
end }
return ui.column { gap = 8, w_full = true, h_full = true,
    ui.text { id = "listing", function() return listing:get() end },
    ui.button { "Plain" },
    ui.checkbox { label = "On", value = ui.state("on", true) },
    ui.checkbox { label = "Off", value = ui.state("off", false) },
    ui.menu { id = "m", open = open,
        anchor = ui.button { id = "opener", "Menu" },
        build = function() return ui.column {
            ui.menu_item { "Word Wrap", checked = wrap },
            ui.menu_item { "Small", radio = true, checked = true },
            ui.menu_item { "Large", radio = true, checked = false },
            ui.menu_item { "Copy" },
        } end },
}
"##;

/// A checkbox, a radio or a checked menu item says whether it is on; what cannot be checked says
/// nothing (nil), so a script can tell an already chosen option.
#[test]
fn controls_carry_checked_where_they_can_be_checked() {
    let mut h = Harness::inline(CHECKED);
    h.frames(3);
    run(&mut h, "scan");
    let listing = h.text("listing");
    for want in [
        "button:Plain:nil",
        "checkbox:On:true",
        "checkbox:Off:false",
        "menuitemcheckbox:Word Wrap:true",
        "menuitemradio:Small:true",
        "menuitemradio:Large:false",
        "menuitem:Copy:nil",
    ] {
        assert!(listing.contains(want), "{want} in {listing}");
    }
}

/// With the app at the back (another Space, behind other apps) a menu wanted open is held, not shown
/// nor hit: its items are no controls on screen, and pressing one lands nowhere.
#[test]
fn a_held_popups_items_are_not_listed_and_press_nothing() {
    let mut h = Harness::inline(APP);
    h.ui.set_popup_screens(vec![zgx::scene::Rect::new(0., 0., 1000., 800.)]);
    h.frames(3);
    h.click("opener");
    h.frames(3);
    run(&mut h, "scan");
    assert!(
        h.text("listing").contains("menuitem:Copy"),
        "{}",
        h.text("listing")
    );
    // To the back: held.
    h.ui.set_app_active(false);
    h.frames(2);
    run(&mut h, "scan");
    let listing = h.text("listing");
    assert!(!listing.contains("menuitem:Copy"), "{listing}");
    assert!(!listing.contains("menu:"), "{listing}");
    // A handle taken while it showed is none to press now.
    h.ui.set_app_active(true);
    h.frames(2);
    run(&mut h, "scan");
    assert!(h.text("listing").contains("menuitem:Copy"));
    // Shown again, it takes the press (its item then flashes, as a menu in a window does).
    run(&mut h, "press_menu");
    assert_eq!(h.text("result"), "true");
}

#[test]
fn pressing_a_held_popups_item_returns_false_and_runs_nothing() {
    let mut h = Harness::inline(APP);
    h.ui.set_popup_screens(vec![zgx::scene::Rect::new(0., 0., 1000., 800.)]);
    h.frames(3);
    h.click("opener");
    h.frames(3);
    run(&mut h, "scan");
    h.ui.set_app_active(false);
    h.frames(2);
    run(&mut h, "press_menu");
    assert_eq!(h.text("result"), "false");
    assert_eq!(h.text("log"), "none", "nothing ran");
}

/// Where menus are windows of their own (`Ui::set_popup_screens`), an item pressed through
/// `ui.press` flashes, closes its menu, then runs: the press lands on the item in the popup (it is
/// part of the window's scene), not outside it.
#[test]
fn a_press_on_an_item_of_a_menu_in_its_own_window_runs_it() {
    let mut h = Harness::inline(APP);
    h.ui.set_popup_screens(vec![zgx::scene::Rect::new(0., 0., 1000., 800.)]);
    h.frames(3);
    h.click("opener");
    h.frames(3);
    run(&mut h, "scan");
    run(&mut h, "press_menu");
    assert_eq!(h.text("result"), "true");
    // (The item flashes, then its menu closes, then it runs: as time passes.)
    assert!(
        h.wait_until(|h| h.text("log") == "copied"),
        "the item's handler ran: {}",
        h.text("log")
    );
}

#[cfg(feature = "inspect")]
/// `windows` lists the popups a window wants open, each with its role, its owner window and
/// whether it shows or is held back (the app at the back).
#[test]
fn inspect_windows_lists_popups_with_their_owner_and_whether_they_show() {
    use zgx_script::inspect::{Reply, Session};
    let mut h = Harness::inline(APP);
    h.ui.set_popup_screens(vec![zgx::scene::Rect::new(0., 0., 1000., 800.)]);
    h.frames(3);
    h.click("opener");
    h.frames(3);
    let session = Session::new(&h.ui, h.handle.clone(), &h.host, None);
    let windows = |session: &Session| match session.answer(r#"{"cmd":"windows"}"#) {
        Reply::Now(json) => json,
        Reply::Later(_) => panic!("windows answers at once"),
    };
    let shown = windows(&session);
    assert!(shown.contains("popup") && shown.contains("menu"), "{shown}");
    assert!(
        shown.contains("shown") && shown.contains("owner main"),
        "{shown}"
    );
    assert!(shown.contains("\"shown\":true"), "{shown}");
    h.ui.set_app_active(false);
    h.frames(2);
    let held = windows(&session);
    assert!(
        held.contains("held") && held.contains("\"held\":true"),
        "{held}"
    );
}
