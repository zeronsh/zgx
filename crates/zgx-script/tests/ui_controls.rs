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
