//! A long column streamed into, in a scroll view and in a virtual list's row,
//! lays out as one built whole: what keeps its children between layouts
//! (`Scene::column_end`) sees these parents too, and what they ask of it.
mod common;
use common::Harness;

/// `layout` holding the column; ids `row1`, `row2`… Buttons append a row, or
/// grow the last.
fn app(layout: &str, rows: usize, words: &str) -> String {
    format!(
        r##"
local rows = ui.state("rows", {{}})
local seed = {{}}
for i = 1, {rows} do seed[i] = {{ id = i, words = "row " .. i }} end
seed[{rows}].words = seed[{rows}].words .. "{words}"
rows:set(seed)
local function column()
    return ui.column {{
        id = "col", w_full = true, gap = 2.5,
        ui.each(rows, function(r)
            return ui.text {{ id = "row" .. r.id, w_full = true, r.words }}
        end, {{ key = function(r) return r.id end }}),
    }}
end
return ui.column {{
    w = 280,
    ui.row {{
        ui.button {{ id = "append", "+", on_click = function()
            local n = #rows:get() + 1
            rows:push({{ id = n, words = "row " .. n }})
        end }},
        ui.button {{ id = "grow", "g", on_click = function()
            local n = #rows:get()
            rows:set_key({{ n, "words" }}, rows:get()[n].words .. " and it goes on and on, on and on")
        end }},
    }},
    {layout},
}}
"##
    )
}
const SCROLL: &str = r#"ui.scroll { id = "view", w_full = true, h = 300, column() }"#;
const LIST: &str = r#"ui.list {
        id = "view", w_full = true, h = 600,
        data = ui.state("parts", { { id = "a" }, { id = "b" } }),
        key = function(r) return r.id end,
        measured = true,
        height = 40,
        row = function(r)
            if r.id == "a" then return ui.text { id = "above", "above it" } end
            return column()
        end,
    }"#;

fn same(h: &mut Harness, rows: usize, words: &str, layout: &str, what: &str) {
    let mut fresh = Harness::inline(&app(layout, rows, words));
    fresh.frames(3);
    assert_eq!(h.error(), None);
    assert_eq!(h.bounds("col"), fresh.bounds("col"), "{what}: the column");
    for row in 1..=rows {
        let id = format!("row{row}");
        assert_eq!(h.bounds(&id), fresh.bounds(&id), "{what}: {id}");
    }
}

fn streams(layout: &str) {
    let tail = " and it goes on and on, on and on";
    let mut h = Harness::inline(&app(layout, 70, ""));
    h.frames(3);
    let (mut rows, mut words) = (70, String::new());
    for step in 0..12 {
        if step % 3 == 2 {
            h.click("grow");
            words.push_str(tail);
        } else {
            h.click("append");
            rows += 1;
            words.clear();
        }
        h.frames(2);
        same(&mut h, rows, &words, layout, &format!("step {step}"));
    }
    // Scrolled, it goes on the same.
    h.scroll("view", 200.0);
    h.frames(2);
    h.click("append");
    rows += 1;
    h.frames(2);
    assert_eq!(h.error(), None);
    let top = h.bounds("row1").y;
    let next = h.bounds("row2").y;
    assert!(next > top, "rows in order after a scroll: {top} {next}");
    assert_eq!(h.text(&format!("row{rows}")), format!("row {rows}"));
}

#[test]
fn a_long_column_in_a_scroll_view_streams_as_one_built_whole() {
    streams(SCROLL);
}

#[test]
fn a_long_column_in_a_list_row_streams_as_one_built_whole() {
    streams(LIST);
}
