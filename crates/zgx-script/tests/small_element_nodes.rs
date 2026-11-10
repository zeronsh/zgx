//! What a small element costs in scene nodes, and that what it no longer
//! makes changes nothing else: a `ui.show` showing nothing is its region
//! alone (no empty box in it).

mod common;

use common::Harness;

/// The nodes from `id`'s node down (it included).
fn nodes_under(h: &mut Harness, id: &str) -> usize {
    let node = h.find(id).unwrap_or_else(|| panic!("no `{id}`"));
    let scene = h.ui.scene.borrow();
    let (mut n, mut stack) = (0, vec![node]);
    while let Some(at) = stack.pop() {
        n += 1;
        stack.extend(scene.children(at).iter().copied());
    }
    n
}

const SHOWS: &str = r#"
local on = ui.state("on", false)
local resized = ui.state("resized", 0)
local r = ui.ref()
local order = ui.state("order", { "a", "b", "c" })
local function open(): boolean
    return on:get()
end
return ui.column {
    ui.button { id = "toggle", on_click = function() on:set(not on:peek()) end, "Toggle" },
    ui.button {
        id = "reverse",
        on_click = function()
            local o = order:peek()
            order:set({ o[3], o[2], o[1] })
        end,
        "Reverse",
    },
    ui.row { id = "gapped", gap = 10, ui.text { id = "left", "L" }, ui.show(open, ui.text { "M" }), ui.text { id = "right", "R" } },
    ui.column {
        id = "case",
        ui.text { "mark" },
        ui.show(open, ui.button {
            id = "inside",
            ref = r,
            on_resize = function()
                resized:set(resized:peek() + 1)
            end,
            on_click = function()
                on:set(false)
            end,
            "Inside",
        }),
    },
    ui.text { id = "measured", function() return `w={(r.width or 0) > 0} n={resized:get() > 0}` end },
    ui.show(function() return not on:get() end, ui.text { id = "closed", "closed" }, ui.text { id = "opened", "opened" }),
    ui.column {
        id = "rows",
        ui.each(order, function(k)
            local own = ui.use_state(`kept {k}`)
            return ui.row { ui.text { function() return own:get() end }, ui.show(open, ui.text { " +" }) }
        end, { key = function(k) return k end }),
    },
    ui.presence(function(): string?
        return if on:get() then "p" else nil
    end, function()
        return ui.column { ui.show(true, ui.text { id = "entered", "Entered" }) }
    end, { enter = { opacity = { 0, 1 }, duration = 100 }, exit = { opacity = { 1, 0 }, duration = 100 } }),
}
"#;

#[test]
fn a_show_showing_nothing_is_its_region_alone() {
    let mut h = Harness::inline(SHOWS);
    h.settle();
    // The column, its text and the show's region: no empty box in it.
    assert_eq!(nodes_under(&mut h, "case"), 3);
    h.click("toggle");
    h.settle();
    // What it shows (a button and its label), in the region.
    assert!(h.find("inside").is_some());
    let inside = nodes_under(&mut h, "inside");
    assert_eq!(nodes_under(&mut h, "case"), 3 + inside);
    h.click("toggle");
    h.settle();
    assert_eq!(nodes_under(&mut h, "case"), 3, "closed again, nothing left");
}

#[test]
fn a_show_showing_nothing_takes_no_gap() {
    let mut h = Harness::inline(SHOWS);
    h.settle();
    let (left, right) = (h.bounds("left"), h.bounds("right"));
    assert!(
        (right.x - (left.x + left.width) - 10.).abs() < 0.01,
        "one gap: {left:?} {right:?}"
    );
    h.click("toggle");
    h.settle();
    let (left, right) = (h.bounds("left"), h.bounds("right"));
    assert!(
        right.x - (left.x + left.width) > 20.,
        "M and two gaps between: {left:?} {right:?}"
    );
}

#[test]
fn what_a_show_shows_is_named_hit_measured_and_told_its_size() {
    let mut h = Harness::inline(SHOWS);
    h.settle();
    assert!(h.find("role:button \"Inside\"").is_none());
    assert_eq!(h.text("measured"), "w=false n=false");
    for _ in 0..2 {
        h.click("toggle");
        h.settle();
        assert!(
            h.find("role:button \"Inside\"").is_some(),
            "named for assistive technology"
        );
        assert_eq!(h.text("measured"), "w=true n=true", "its ref and on_resize");
        // Hit where it is: its click closes it.
        h.click("inside");
        h.settle();
        assert!(h.find("inside").is_none());
    }
}

#[test]
fn a_show_shows_its_otherwise() {
    let mut h = Harness::inline(SHOWS);
    h.settle();
    assert!(h.find("closed").is_some() && h.find("opened").is_none());
    h.click("toggle");
    h.settle();
    assert!(h.find("closed").is_none() && h.find("opened").is_some());
}

#[test]
fn keyed_rows_with_shows_keep_their_state_as_they_move() {
    let mut h = Harness::inline(SHOWS);
    h.settle();
    let rows = |h: &mut Harness| {
        h.texts()
            .into_iter()
            .filter(|t| t.starts_with("kept"))
            .collect::<Vec<_>>()
    };
    assert_eq!(rows(&mut h), ["kept a", "kept b", "kept c"]);
    h.click("reverse");
    h.settle();
    assert_eq!(rows(&mut h), ["kept c", "kept b", "kept a"]);
    h.click("toggle");
    h.settle();
    assert_eq!(h.texts().iter().filter(|t| *t == " +").count(), 3);
    h.click("reverse");
    h.settle();
    assert_eq!(rows(&mut h), ["kept a", "kept b", "kept c"]);
}

#[test]
fn a_show_in_a_presence_enters_and_leaves_with_it() {
    let mut h = Harness::inline(SHOWS);
    h.settle();
    assert!(h.find("entered").is_none());
    h.click("toggle");
    h.frames(2);
    assert!(h.find("entered").is_some());
    h.frames(30);
    h.click("toggle");
    h.frames(2);
    assert!(h.find("entered").is_some(), "still there as it leaves");
    h.frames(30);
    h.settle();
    assert!(h.find("entered").is_none());
}

/// A row whose hover restyles it (as a sidebar section's header does), two
/// shows in it trading places as the pointer comes and goes: the one closed
/// again still takes no place (its region's own style, applied again as the
/// row restyles, doesn't bring it back into layout).
#[test]
fn a_show_closed_again_in_a_restyled_row_takes_no_gap() {
    let mut h = Harness::inline(
        r##"
local hovered = ui.state("hovered", false)
local editing = ui.state("editing", false)
local menu_open = ui.state("menu_open", false)
local menu = ui.popover { open = menu_open, anchor = ui.button { id = "dots", "…" }, build = function() return ui.text { "menu" } end }
return ui.column {
    ui.row { id = "away", w = 200, h = 30, ui.text { "away" } },
    ui.row {
        id = "hdr", w = 240, h = 24, gap = 4, pl = 8, pr = 4, items = "center",
        color = "#888888", hover = { color = "#aaaaaa" }, cursor = "pointer",
        on_hover = function(inside: boolean) hovered:set(inside) end,
        on_click = function() end,
        ui.show(function() return editing:get() end, ui.text { "editing" }, ui.text { id = "name", grow = 1, "Later" }),
        ui.show(function() return not hovered:get() and not menu_open:get() end, ui.row { shrink = 0, pr = 4, ui.text { id = "count", "1" } }),
        ui.show(function() return hovered:get() or menu_open:get() end, menu),
    },
}
"##,
    );
    h.settle();
    let before = h.bounds("count");
    h.hover("hdr");
    h.settle();
    h.hover("away");
    h.settle();
    let after = h.bounds("count");
    assert_eq!(before, after, "the count where it was");
}
