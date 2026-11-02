//! Focus moving into a list's rows scrolls the list to them (a long chat's
//! transcript tabbed through): rows of very different heights, built only
//! around the view, an offset the script holds, and a composer floating
//! over the list's bottom (`scroll_padding`).
mod common;
use common::Harness;
use zgx::input::{Key, Modifiers};

const APP: &str = r#"
local rows = ui.state("rows", (function()
    local list = {}
    for n = 1, 200 do
        -- 40 to 600 points, in no order.
        table.insert(list, { id = n, h = 40 + (n * 137) % 561 })
    end
    return list
end)())
local offset = ui.state("offset", 0)
return ui.overlay {
    w = 400, h = 600,
    ui.list {
        id = "list",
        w = 400, h = 600,
        height = 40,
        measured = true,
        data = rows,
        key = function(row) return row.id end,
        offset = offset,
        scroll_padding = { bottom = 80 },
        row = function(row)
            return ui.column {
                h = row.h,
                ui.button { id = `tool-{row.id}`, w = 120, h = 30, "Ran 2 commands" },
            }
        end,
    },
    -- The composer, over the list's bottom.
    ui.div { id = "composer", absolute = true, bottom = 0, left = 0, w = 400, h = 80 },
    ui.button { id = "far", absolute = true, top = 0, right = 0, w = 10, h = 10,
        on_click = function() offset:set(100000) end },
    ui.text { id = "offset", absolute = true, top = 0, left = 0, w = 1, h = 1,
        function() return tostring(offset:get()) end },
}
"#;

/// The focused node's top and bottom, in window points.
fn focused(h: &mut Harness) -> (f32, f32) {
    let node = h.ui.input.focused().expect("something focused");
    let rect = h.ui.scene.borrow().transformed_bounds(node);
    (rect.y, rect.y + rect.height)
}

#[test]
fn tabbing_back_through_a_list_scrolls_each_row_into_view_clear_of_its_padding() {
    let mut h = Harness::inline(APP);
    h.frames(2);
    // To the end, as a chat rests.
    h.click("far");
    h.frames(3);
    let end: f32 = h.text("offset").parse().unwrap();
    assert!(end > 10_000., "at the end: {end}");
    let mut rows = Vec::new();
    for step in 0..40 {
        h.key_with(
            Key::Tab,
            Modifiers {
                shift: true,
                ..Default::default()
            },
        );
        h.frames(1);
        let (top, bottom) = focused(&mut h);
        let at = h.ui.input.focused();
        let row = (1..=200).find(|n| h.find(&format!("tool-{n}")) == at);
        // Shown, above the composer: 0 to 520 of the list's 600.
        assert!(
            top >= -0.01 && bottom <= 520.01,
            "step {step}: row {row:?} at {top}..{bottom}"
        );
        rows.push(row.expect("a row's control"));
    }
    // Back through the rows one by one, each built as the list came to it.
    // Entered at the last row, as from below a list held whole.
    assert!(
        rows[0] == 200 && rows.windows(2).all(|pair| pair[1] == pair[0] - 1),
        "{rows:?}"
    );
    // (Rows measured as they come in move the estimate above them, so the
    // offset moves less than the rows passed.)
    let now: f32 = h.text("offset").parse().unwrap();
    assert!(now < end - 500., "the offset followed: {now} from {end}");
    // And forward again: each row comes in at the bottom, above the
    // composer, not under it.
    let mut at_bottom = 0;
    for step in 0..10 {
        h.key(Key::Tab);
        h.frames(1);
        let (top, bottom) = focused(&mut h);
        assert!(
            top >= -0.01 && bottom <= 520.01,
            "forward {step}: at {top}..{bottom}"
        );
        at_bottom += usize::from((bottom - 520.).abs() < 0.5);
    }
    assert!(at_bottom > 0, "none came in at the padding's edge");
    assert_eq!(h.error(), None);
}

#[test]
fn focus_without_scroll_leaves_the_list_where_it_is() {
    let mut h = Harness::inline(
        r#"
local offset = ui.state("offset", 0)
local target = nil
local list = ui.list {
    id = "list", w = 400, h = 300, height = 50, count = 100, offset = offset,
    row = function(i)
        return ui.button { id = `row-{i}`, w = 100, h = 50, tostring(i),
            on_mount = function(el) if i == 20 then target = el end end }
    end,
}
return ui.column {
    ui.row {
        ui.button { id = "move", w = 20, h = 20, on_click = function() offset:set(1000) end },
        ui.button { id = "quiet", w = 20, h = 20, on_click = function()
            target:focus({ scroll = false })
        end },
        ui.button { id = "loud", w = 20, h = 20, on_click = function() target:focus() end },
        ui.text { id = "offset", function() return tostring(offset:get()) end },
    },
    list,
}
"#,
    );
    h.frames(2);
    h.click("move");
    h.frames(2);
    // Row 20 is built ahead of the view (overscan), 50 points above it.
    h.click("quiet");
    h.frames(2);
    assert_eq!(h.text("offset"), "1000");
    h.click("move");
    h.frames(2);
    h.click("loud");
    h.frames(2);
    assert_eq!(h.text("offset"), "950");
    assert_eq!(h.error(), None);
}

#[test]
fn a_live_scroll_padding_follows_the_composer_as_it_grows() {
    // The padding from a state: the composer grows taller after the list is
    // made, and a row tabbed to comes in above its new top.
    let mut h = Harness::inline(
        r#"
local composer = ui.state("composer", 80)
ui.command { id = "grow", run = function() composer:set(200) end }
local rows = {}
for i = 1, 40 do rows[i] = { id = i } end
return ui.overlay {
    w = 400, h = 600,
    ui.list {
        id = "list", w = 400, h = 600, height = 50,
        data = ui.state("rows", rows),
        key = function(row) return row.id end,
        scroll_padding = function() return { bottom = composer:get() } end,
        row = function(row)
            return ui.button { id = `b{row.id}`, w = 100, h = 50, tostring(row.id) }
        end,
    },
}
"#,
    );
    h.frames(2);
    assert!(h.host.run_command("grow"));
    h.frames(2);
    h.click("b1");
    // Tab down the rows: each one shown above the 200-point composer.
    for step in 0..12 {
        h.key(Key::Tab);
        h.frames(1);
        let (top, bottom) = focused(&mut h);
        assert!(
            top >= -0.01 && bottom <= 400.01,
            "step {step}: at {top}..{bottom}"
        );
    }
    assert_eq!(h.error(), None);
}
