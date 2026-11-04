//! A `ui.list` builds what shows: rows folded to nothing aren't built, however
//! many of them fit in its viewport.
mod common;

use common::Harness;

/// 2,000 rows, each `height` tall but for the first three, counting builds.
fn list(height: &str) -> String {
    format!(
        r##"local built = ui.shared("built", 0)
        local open = ui.shared("open", false)
        local heights = ui.computed(function()
            local out = {{}}
            for i = 1, 2000 do
                out[i] = if i <= 3 or open:get() then 20 else {height}
            end
            return out
        end)
        return ui.list {{ id = "list", w = 200, h = 400, heights = heights,
            key = function(i) return "row" .. i end,
            row = function(key)
                built:set(built:peek() + 1)
                return ui.div {{ id = key, w_full = true, h_full = true }}
            end,
        }}"##
    )
}
fn built(h: &mut Harness) -> f64 {
    h.settle();
    h.frames(2);
    h.host
        .shared("built", 0.)
        .with_untracked(|v| v.as_f64())
        .unwrap()
}

#[test]
fn rows_folded_to_nothing_are_not_built() {
    for height in ["0", "0.001"] {
        let mut h = Harness::inline(&list(height));
        assert_eq!(h.error(), None);
        assert!(built(&mut h) <= 3., "{height}: the three that show");
        assert!(h.find("row1").is_some());
        assert!(h.find("row4").is_none(), "{height}: folded");
    }
}

#[test]
fn a_row_of_no_height_takes_no_room() {
    let mut h = Harness::inline(
        r##"return ui.list { id = "list", w = 200, h = 400, heights = { 20, 0, 20 },
            key = function(i) return "row" .. i end,
            row = function(key) return ui.div { id = key, w_full = true, h_full = true } end,
        }"##,
    );
    h.settle();
    h.frames(2);
    assert_eq!(h.bounds("row3").y - h.bounds("row1").y, 20.);
}

#[test]
fn rows_opening_are_built_as_they_come_to_show() {
    let mut h = Harness::inline(&list("0.001"));
    assert!(built(&mut h) <= 3.);
    h.host
        .shared("open", false)
        .set(zgx_script::Value::Bool(true));
    let after = built(&mut h);
    assert!(h.find("row4").is_some(), "opened");
    assert!(after < 60., "what fits in the view, no more: {after}");
}
