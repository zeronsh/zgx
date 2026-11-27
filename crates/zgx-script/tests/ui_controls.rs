//! `ui.controls` lists the controls on screen as the accessibility tree has them, popups last,
//! `ui.press` presses one as `zgx inspect click` does and `ui.text_of` reads what it shows; a
//! stale handle is nothing. `el:scroll_info` says how far a scroll view can go.
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

#[test]
fn a_control_says_what_it_is_checked_pressed_and_opens() {
    let mut h = Harness::inline(
        r##"
local out = ui.state("out", "")
ui.command { id = "read", run = function()
    local seen = {}
    for _, c in ui.controls() do
        if c.name == "Agree" or c.name == "Bold" or c.name == "Menu" then
            table.insert(seen, `{c.name}:{tostring(c.checked)}/{tostring(c.pressed)}/{tostring(c.haspopup)}/{tostring(c.expanded)}`)
        end
    end
    out:set(table.concat(seen, "|"))
end }
return ui.column { w = 300, h = 300,
    ui.checkbox { label = "Agree", value = ui.state("agree", true) },
    ui.button { "Bold", pressed = ui.state("bold", false) },
    ui.menu { open = ui.state("open", true), anchor = ui.button { "Menu" },
        build = function() return ui.column { ui.menu_item { "Copy" } } end },
    ui.text { id = "out", function() return out:get() end },
}"##,
    );
    h.frames(3);
    run(&mut h, "read");
    assert_eq!(
        h.text("out"),
        "Agree:true/nil/nil/nil|Bold:nil/false/nil/nil|Menu:nil/nil/menu/true"
    );
}

#[test]
fn text_of_reads_what_a_control_shows() {
    let mut h = Harness::inline(
        r##"
local out = ui.state("out", "")
ui.command { id = "read", run = function()
    for _, c in ui.controls() do
        if c.role == "dialog" then out:set(ui.text_of(c.handle) .. "|" .. ui.text_of(c.handle, 5)) end
    end
end }
ui.command { id = "gone", run = function() out:set("[" .. ui.text_of(1e15) .. "][" .. ui.text_of(-1) .. "]") end }
return ui.column { w = 400, h = 300,
    ui.column { role = "dialog", label = "Notice",
        ui.text { "Heads up" },
        ui.text { "something happened" },
        ui.column { aria_hidden = true, ui.text { "decoration" } },
    },
    ui.text { id = "out", function() return out:get() end },
}"##,
    );
    h.settle();
    h.frames(3);
    run(&mut h, "read");
    assert_eq!(h.text("out"), "Heads up something happened|Heads");
    run(&mut h, "gone");
    assert_eq!(h.text("out"), "[][]");
}

#[test]
fn scroll_info_says_how_far_a_scroll_view_can_go() {
    let mut h = Harness::inline(
        r##"
local out = ui.state("out", "")
local box = ui.ref()
local lst = ui.ref()
local plain = ui.ref()
ui.command { id = "read", run = function()
    local info = box:get():scroll_info()
    local list = lst:get():scroll_info()
    out:set(`{info.offset}/{info.range}/{tostring(info.at_start)}/{tostring(info.at_end)}|{list.range}/{tostring(list.at_start)}|{tostring(plain:get():scroll_info())}`)
end }
return ui.column { w = 200, h = 300,
    ui.scroll { ref = box, id = "s", w = 200, h = 100,
        ui.column { h = 400, ui.text { "tall" } } },
    ui.list { ref = lst, count = 100, height = 20, w = 200, h = 100,
        row = function(i) return ui.text("row " .. i) end },
    ui.text { ref = plain, id = "out", function() return out:get() end },
}"##,
    );
    h.settle();
    h.frames(3);
    run(&mut h, "read");
    assert_eq!(h.text("out"), "0/300/true/false|1900/true|nil");
    // Scrolled to its end, it can go no further down.
    h.scroll("s", 1000.);
    h.frames(3);
    run(&mut h, "read");
    assert_eq!(h.text("out"), "300/300/false/true|1900/true|nil");
}

#[test]
fn a_cut_menu_says_whether_it_can_scroll() {
    let mut h = Harness::inline(
        r##"
local open = ui.state("open", true)
local items = {}
for i = 1, 20 do table.insert(items, ui.menu_item { id = if i == 1 then "first" else nil, "Item " .. i }) end
local out = ui.state("out", "")
ui.command { id = "close", run = function() open:set(false) end }
ui.command { id = "read", run = function()
    out:set("none")
    for _, c in ui.controls() do
        if c.role == "menu" then out:set(`{tostring(c.at_start)}/{tostring(c.at_end)}/{tostring(c.scroll_range ~= nil and c.scroll_range > 0)}`) end
    end
end }
return ui.column { w = 300, h = 300,
    ui.menu { open = open, anchor = ui.button { "Open" }, max_h = 60, table.unpack(items) },
    ui.text { id = "out", function() return out:get() end },
}"##,
    );
    h.settle();
    h.frames(4);
    run(&mut h, "read");
    assert_eq!(h.text("out"), "true/false/true");
    // Scrolled to its end it says so; closed, it is no control.
    h.scroll("first", 5000.);
    h.frames(3);
    run(&mut h, "read");
    assert_eq!(h.text("out"), "false/true/true");
    run(&mut h, "close");
    run(&mut h, "read");
    assert_eq!(h.text("out"), "none");
}

/// Nothing of them is made for a script that doesn't name them.
#[test]
fn they_are_made_only_when_named() {
    let mut h = Harness::inline(
        r#"return ui.text { id = "made", tostring(rawget(ui, "controls")) .. tostring(rawget(ui, "press")) .. tostring(rawget(ui, "text_of")) }"#,
    );
    assert_eq!(h.error(), None);
    assert_eq!(h.text("made"), "nilnilnil");
}

/// Two hosts on one runtime: each lists its own windows.
#[test]
fn a_host_lists_its_own_windows() {
    const LISTS: &str = r##"
local out = ui.state("out", "")
ui.command { id = "scan", run = function()
    local names = {}
    for _, c in ui.controls() do table.insert(names, c.name) end
    out:set(table.concat(names, "|"))
end }
return ui.column { ui.button { "Mine" }, ui.text { id = "out", function() return out:get() end } }
"##;
    let mut h = Harness::inline(LISTS);
    h.frames(3);
    let other = zgx_script::ScriptHost::inline(&h.ui.runtime, "other", LISTS);
    let shown = zgx::widgets::Ui::with_runtime(h.ui.runtime.clone(), 100., 100.);
    other.window_controls(&shown);
    // The other's is the active one, and the last given.
    h.ui.set_window_active(false);
    run(&mut h, "scan");
    assert_eq!(h.text("out"), "Mine");
}

/// A host that lists, presses and reads; a Rust button beside its view and
/// one in a `ui.host_view` inside it, each counting its presses.
fn beside_rust() -> (Harness, std::rc::Rc<std::cell::Cell<i32>>) {
    use zgx::compose::prelude::*;
    let pressed = std::rc::Rc::new(std::cell::Cell::new(0));
    let trusted = {
        let pressed = pressed.clone();
        move |label: &str| {
            let pressed = pressed.clone();
            button()
                .on_click(move || pressed.set(pressed.get() + 1))
                .child(text(label.to_owned()))
        }
    };
    let inside = trusted.clone();
    zgx_script::host_view("controls test trusted", move || inside("Keep template"));
    let h = Harness::wrapped(
        move |runtime| {
            zgx_script::ScriptHost::inline(
                runtime,
                "app",
                r##"
local out, listing = ui.state("out", ""), ui.state("listing", "")
local given = ui.state("given", 0)
local menu = ui.state("menu", false)
local handles = {}
ui.command { id = "scan", run = function()
    local names = {}
    handles = {}
    for _, c in ui.controls() do table.insert(names, c.name); handles[c.name] = c.handle end
    listing:set(table.concat(names, "|"))
end }
ui.command { id = "press_mine", run = function() out:set(tostring(ui.press(handles["Mine"] or -1))) end }
ui.command { id = "press_item", run = function() out:set(tostring(ui.press(handles["Item"] or -1))) end }
ui.command { id = "press_given", run = function()
    local ok, e = pcall(ui.press, given:get())
    out:set(ok and tostring(e) or "refused")
end }
ui.command { id = "read_given", run = function()
    local ok, e = pcall(ui.text_of, given:get())
    out:set(ok and e or "refused")
end }
ui.command { id = "read_box", run = function() out:set(ui.text_of(given:get())) end }
return ui.column { gap = 8, w = 400, h = 300,
    ui.text { id = "listing", function() return listing:get() end },
    ui.text { id = "out", function() return out:get() end },
    ui.button { "Mine", on_click = function() out:set("mine") end },
    ui.column { id = "box", role = "dialog", label = "Box", w = 200, h = 60,
        ui.text { "own text" }, ui.host_view { name = "controls test trusted" } },
    ui.menu { id = "m", open = menu,
        anchor = ui.button { id = "opener", "Opener", on_click = function() menu:set(true) end },
        build = function() return ui.column { ui.menu_item { "Item", on_click = function() out:set("item") end } } end },
}"##,
            )
        },
        // Above it, so both show.
        move |view| column().child(trusted("Rust button")).child(view),
    );
    (h, pressed)
}
/// The node of the control `label` says it is, as the host knows it.
fn handle_of(h: &Harness, label: &str) -> f64 {
    zgx_script::driver::controls(&h.ui, None)
        .into_iter()
        .find(|c| c.name == label)
        .expect(label)
        .handle as f64
}
fn give(h: &mut Harness, label: &str) {
    let handle = handle_of(h, label);
    h.host
        .set_state("given", zgx_script::Value::Num(handle))
        .unwrap();
    h.frames(2);
}

/// A script reaches the whole window, the host's controls included.
#[test]
fn a_script_reaches_the_whole_window() {
    let (mut h, pressed) = beside_rust();
    h.frames(3);
    run(&mut h, "scan");
    let listing = h.text("listing");
    assert!(
        listing.contains("Rust button") && listing.contains("Keep template"),
        "{listing}"
    );
    give(&mut h, "Rust button");
    run(&mut h, "press_given");
    assert_eq!(h.text("out"), "true");
    assert_eq!(pressed.get(), 1);
}

/// A script's press is not a person's: what it presses can't take the
/// pointer, even just after a person pressed (their activation still live),
/// while a person's own press of the same control does.
#[test]
fn a_scripted_press_takes_no_pointer_lock() {
    let mut h = Harness::new(|runtime| {
        zgx_script::ScriptHost::inline(
            runtime,
            "app",
            r##"
local out, locked = ui.state("out", ""), ui.state("locked", false)
local play = ui.ref()
ui.command { id = "press_play", run = function()
    for _, c in ui.controls() do
        if c.name == "Play" then out:set(tostring(ui.press(c.handle))) end
    end
end }
return ui.column { w = 300, h = 200,
    ui.button { id = "play", ref = play, "Play",
        on_click = function() play:get():lock_pointer() end,
        on_pointer_lock = function(on) locked:set(on) end },
    ui.text { id = "out", function() return `{out:get()} {locked:get()}` end },
}"##,
        )
    });
    h.frames(3);
    // A person's press or key a moment ago: their activation is live.
    h.ui.input.note_gesture();
    run(&mut h, "press_play");
    assert_eq!(h.text("out"), "true false", "pressed, and no lock");
    assert_eq!(h.ui.input.pointer_lock(&h.ui.scene.borrow()), None);
    // A person's press of the same button, as a host hands it on (noted,
    // then dispatched).
    h.ui.input.note_gesture();
    h.click("play");
    h.frames(2);
    assert_eq!(h.text("out"), "true true", "a person's press locks it");
}
