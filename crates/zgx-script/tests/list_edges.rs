//! A list's start coming in reach (`on_start`), whether it rests at its end
//! (`at_end`), and its rows in view staying put as an earlier page is added
//! above them.
mod common;
use common::Harness;

const APP: &str = r#"
-- Rows 101 to 140; each earlier page adds the ten before the first.
local rows = ui.state("rows", (function()
    local list = {}
    for n = 101, 140 do
        table.insert(list, { id = n })
    end
    return list
end)())
local offset = ui.state("offset", 400)
local at_end = ui.state("at_end", false)
local pages = ui.state("pages", 0)
return ui.column {
    ui.text { id = "state", function()
        return `{pages:get()} {at_end:get()} {#rows:get()}`
    end },
    ui.row {
        ui.button { id = "to_end", w = 40, h = 20, "e", on_click = function() offset:set(600) end },
        ui.button { id = "near_start", w = 40, h = 20, "s", on_click = function() offset:set(30) end },
    },
    ui.list {
        id = "list",
        h = 200,
        w = 200,
        height = 20,
        measured = true,
        data = rows,
        key = function(row) return row.id end,
        offset = offset,
        at_end = at_end,
        start_distance = 40,
        on_start = function()
            local list = rows:peek()
            local first = list[1].id
            if first <= 81 then
                return
            end
            pages:set(pages:peek() + 1)
            local earlier = {}
            for n = first - 10, first - 1 do
                table.insert(earlier, { id = n })
            end
            for _, row in list do
                table.insert(earlier, row)
            end
            rows:set(earlier)
        end,
        row = function(row)
            return ui.text { id = `row-{row.id}`, h = 20, tostring(row.id) }
        end,
    },
}
"#;

#[test]
fn an_earlier_page_comes_in_above_and_the_view_stays_on_its_rows() {
    let mut h = Harness::inline(APP);
    h.frames(3);
    // 40 rows of 20 points in 200: the end is at 600; 400 is short of it.
    assert_eq!(h.text("state"), "0 false 40");
    // Scrolled to the end: at its end.
    h.click("to_end");
    h.frames(2);
    assert_eq!(h.text("state"), "0 true 40");
    // Up to 30 points from the start: an earlier page comes in above. Row
    // 102 started 20 points down, so 10 points of it were above the view;
    // with ten rows of 20 points above it now, it is 10 points above still.
    h.click("near_start");
    h.frames(4);
    assert_eq!(h.text("state"), "1 false 50");
    let shown = |h: &mut Harness, id: &str| {
        let node = h.find(id).unwrap();
        h.ui.scene.borrow().transformed_bounds(node)
    };
    let list = shown(&mut h, "list");
    let row = shown(&mut h, "row-102");
    assert_eq!(row.y - list.y, -10.);
    assert_eq!(h.error(), None);
}

/// Where an offset of `math.huge`, `-math.huge` or NaN leaves `list`, a
/// list or scroll view of rows `row-1` to `row-60`: whether the first and
/// the last row show.
fn ends_shown(list: &str) -> Vec<(bool, bool)> {
    let mut h = Harness::inline(&format!(
        r#"local offset = ui.state("offset", 300)
        ui.command {{ id = "huge", run = function() offset:set(math.huge) end }}
        ui.command {{ id = "minus", run = function() offset:set(-math.huge) end }}
        ui.command {{ id = "nan", run = function() offset:set(0 / 0) end }}
        local rows = {{}}
        for n = 1, 60 do rows[n] = {{ id = n }} end
        local function row(r)
            local n = if type(r) == "number" then r else r.id
            return ui.text {{ id = `row-{{n}}`, h = 20, tostring(n) }}
        end
        return {list}"#
    ));
    h.frames(3);
    ["huge", "minus", "huge", "nan"]
        .iter()
        .map(|command| {
            assert!(h.host.run_command(command));
            h.frames(5);
            (in_view(&mut h, "row-1"), in_view(&mut h, "row-60"))
        })
        .collect()
}

/// Whether `id` is built and inside the 200-point view at the window's top.
fn in_view(h: &mut Harness, id: &str) -> bool {
    h.find(id).is_some_and(|node| {
        let rect = h.ui.scene.borrow().transformed_bounds(node);
        rect.y >= -0.5 && rect.y + rect.height <= 200.5
    })
}

/// `math.huge` is past the end, so every list kind shows its end, as a
/// scroll view does; `-math.huge` and NaN, its start.
#[test]
fn an_infinite_offset_is_the_end_in_every_list_kind_as_in_a_scroll_view() {
    let want = vec![(false, true), (true, false), (false, true), (true, false)];
    for (kind, list) in [
        (
            "fixed",
            "ui.list { h = 200, height = 20, count = 60, offset = offset, row = row }",
        ),
        (
            "keyed",
            "ui.list { h = 200, height = 20, data = ui.state('rows', rows), \
             key = function(r) return r.id end, offset = offset, row = row }",
        ),
        (
            "measured",
            "ui.list { h = 200, height = 20, measured = true, count = 60, \
             offset = offset, row = row }",
        ),
        (
            "measured and keyed",
            "ui.list { h = 200, height = 20, measured = true, data = ui.state('rows', rows), \
             key = function(r) return r.id end, offset = offset, row = row }",
        ),
        (
            "scroll",
            "ui.scroll { h = 200, w = 200, offset = offset, ui.column((function() \
             local out = {} for n = 1, 60 do out[n] = row(n) end return out end)()) }",
        ),
    ] {
        assert_eq!(ends_shown(list), want, "{kind}");
    }
}
