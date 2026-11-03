//! A reload builds the interface again: what had the keyboard keeps it.
//! A plain focusable node anywhere (not only the first), and a keyed
//! `ui.list`'s row, found again by its role and name wherever the list
//! builds it.
mod common;
use common::Harness;
use zgx_script::ScriptHost;

const APP: &str = r#"
local items = {}
for i = 1, 50 do
    items[i] = { id = `r{i}` }
end
local data = ui.state("probe_items", items)
local picked = ui.state("picked", "")
return ui.column {
    w_full = true,
    h_full = true,
    ui.row { id = "plain", role = "button", label = "Plain", focusable = true, w = 100, h = 30, ui.text { "Plain" } },
    ui.row { id = "second", role = "button", label = "Second", focusable = true, w = 100, h = 30, ui.text { "Second" } },
    ui.list {
        id = "deletes", data = ui.state("deletes", { { id = "a" }, { id = "b" }, { id = "c" } }),
        key = function(it: any): string return it.id end,
        height = 30, w = 300, h = 100,
        row = function(it: any): Node
            return ui.row { ui.text { it.id, w = 40 },
                ui.button { label = "Delete", w = 60, h = 26, on_click = function() picked:set(it.id) end, "Delete" } }
        end,
    },
    ui.column {
        ui.each(ui.state("removes", { "x", "y", "z" }), function(it: string): Node
            return ui.row { ui.text { it, w = 40 },
                ui.button { label = "Remove", w = 60, h = 26, on_click = function() picked:set(it) end, "Remove" } }
        end),
    },
    ui.text { id = "picked", "{picked}" },
    ui.list {
        id = "list",
        data = data,
        key = function(it: any): string return it.id end,
        height = 26,
        w = 300,
        h = 300,
        row = function(it: any): Node
            return ui.row { role = "treeitem", label = it.id, focusable = true, w_full = true, h = 26, ui.text { it.id } }
        end,
    },
}
"#;

/// Focus `target` with a click, reload, and say what had focus before and
/// after (its line in the tree).
fn across_a_reload(test: &str, target: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("zgx-reload-focus-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("app.luau");
    std::fs::write(&path, APP).unwrap();
    let mut h = Harness::new(move |runtime| ScriptHost::new(runtime, path.clone()));
    h.ui.scene.borrow_mut().resize(800., 900.);
    h.frames(10);
    h.click(target);
    h.frames(10);
    let focused = |h: &mut Harness| {
        let tree = h.tree();
        tree.lines()
            .find(|l| l.contains("focused"))
            .map(|l| {
                l.split_whitespace()
                    .skip(1)
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default()
    };
    let before = focused(&mut h);
    h.host.reload().expect("it reloads");
    h.frames(30);
    (before, focused(&mut h))
}

#[test]
fn a_reload_keeps_focus_on_a_plain_node_not_only_the_first() {
    let (before, after) = across_a_reload("second", "role:button \"Second\"");
    assert!(before.contains("\"Second\""), "{before}");
    assert_eq!(after, before);
}

#[test]
fn a_reload_keeps_focus_on_a_keyed_list_row() {
    let (before, after) = across_a_reload("row", "role:treeitem \"r3\"");
    assert!(before.contains("\"r3\""), "{before}");
    assert_eq!(after, before);
}

/// Focus the `nth` (from 1) of the nodes `target` names alike, reload, press it
/// (Enter): which row it was, by what it picks.
fn alike_across_a_reload(test: &str, target: &str, nth: usize) -> String {
    let dir = std::env::temp_dir().join(format!("zgx-reload-focus-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("app.luau");
    std::fs::write(&path, APP).unwrap();
    let mut h = Harness::new(move |runtime| ScriptHost::new(runtime, path.clone()));
    h.ui.scene.borrow_mut().resize(800., 900.);
    h.frames(10);
    h.click(&format!("{target}[{nth}]"));
    h.frames(10);
    h.host.reload().expect("it reloads");
    h.frames(30);
    h.key(zgx::input::Key::Enter);
    h.frames(2);
    h.text("picked")
}

#[test]
fn a_reload_keeps_focus_on_the_second_of_three_alike_row_buttons() {
    // Every row has a "Delete": focus goes back to the second row's, by
    // the row, not the first that is named so.
    assert_eq!(
        alike_across_a_reload("list-alike", "role:button \"Delete\"", 2),
        "b"
    );
    // Rows of a `ui.each` alike too: by their place.
    assert_eq!(
        alike_across_a_reload("each-alike", "role:button \"Remove\"", 2),
        "y"
    );
}

#[test]
fn a_reload_scrolls_a_list_back_to_its_focused_row() {
    // Row r30, scrolled to and focused; the reload builds the list from
    // its top, where r30 isn't built: the list is scrolled to it, and it
    // has focus again.
    let dir = std::env::temp_dir().join(format!("zgx-reload-focus-{}-far", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("app.luau");
    std::fs::write(&path, APP).unwrap();
    let mut h = Harness::new(move |runtime| ScriptHost::new(runtime, path.clone()));
    h.ui.scene.borrow_mut().resize(800., 900.);
    h.frames(10);
    h.scroll("list", 26. * 25.);
    h.frames(5);
    h.click("role:treeitem \"r30\"");
    h.frames(5);
    let focused = |h: &mut Harness| {
        h.tree()
            .lines()
            .find(|l| l.contains("focused"))
            .map(str::to_string)
            .unwrap_or_default()
    };
    assert!(focused(&mut h).contains("\"r30\""), "{}", focused(&mut h));
    h.host.reload().expect("it reloads");
    h.frames(30);
    assert!(focused(&mut h).contains("\"r30\""), "{}", focused(&mut h));
}

const PALETTE: &str = r#"
local open = ui.state("open", false)
ui.command { id = "open", title = "Open", run = function() open:set(true) end }
return ui.column { w = 400, h = 300,
    ui.button { id = "first", "First" }, ui.button { id = "second", "Second" },
    ui.input { id = "field", value = "" },
    ui.modal { open = open, build = function() return ui.input { id = "search", value = "" } end } }
"#;

/// The palette, from a file so it reloads.
fn palette(test: &str) -> Harness {
    let dir = std::env::temp_dir().join(format!("zgx-reload-focus-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("app.luau");
    std::fs::write(&path, PALETTE).unwrap();
    let h = Harness::new(move |runtime| ScriptHost::new(runtime, path.clone()));
    h.frames(10);
    h
}
fn focused_is(h: &mut Harness, id: &str) -> bool {
    let node = h.find(id);
    node.is_some() && h.ui.input.focused() == node
}

/// Zeron's "Escape randomly focuses an element when you close the command
/// palette": a reload dropped focus to the first Tab stop, and closing the
/// palette then gave it back there.
#[test]
fn a_reload_then_a_palette_opened_and_closed_leaves_focus_where_it_was() {
    let mut h = palette("palette-after");
    h.click("field");
    h.frames(5);
    h.host.reload().expect("it reloads");
    h.frames(10);
    assert!(focused_is(&mut h, "field"), "after the reload");
    assert!(h.host.run_command("open"));
    h.frames(10);
    assert!(focused_is(&mut h, "search"), "the palette takes focus");
    h.key(zgx::input::Key::Escape);
    h.frames(10);
    assert!(focused_is(&mut h, "field"), "after Escape");
}

/// A reload while the palette is open: closing it gives focus back to what
/// had it before it opened, found again in what the reload built.
#[test]
fn a_reload_while_a_palette_is_open_still_closes_back_to_where_focus_was() {
    let mut h = palette("palette-during");
    h.click("field");
    h.frames(5);
    assert!(h.host.run_command("open"));
    h.frames(10);
    assert!(focused_is(&mut h, "search"), "the palette takes focus");
    h.host.reload().expect("it reloads");
    h.frames(10);
    assert!(focused_is(&mut h, "search"), "still in the palette");
    h.key(zgx::input::Key::Escape);
    h.frames(10);
    assert!(focused_is(&mut h, "field"), "after Escape");
}
