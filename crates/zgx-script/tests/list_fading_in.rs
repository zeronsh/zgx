//! A list scrolled by the view around it (`virtualize = "ancestor"`) that a
//! presence swaps in with an opacity `enter` builds the rows in view as it
//! starts to fade in, as one swapped in without the fade does.
mod common;

use common::Harness;

fn swapped_in(enter: &str) -> String {
    format!(
        r##"local shown = ui.state("shown", "none")
local items = {{}}
for i = 1, 40 do
    table.insert(items, {{ id = "row_" .. i }})
end
local data = ui.state("items", items)
return ui.scroll {{ w = 400, h = 300,
    ui.column {{ w_full = true,
        ui.button {{ id = "go", "Show", h = 40, on_click = function() shown:set("list") end }},
        ui.presence(shown, function(name)
            if name == "none" then
                return ui.text {{ "Nothing yet" }}
            end
            return ui.list {{
                data = data,
                key = function(item) return item.id end,
                heights = function()
                    local out = {{}}
                    for i = 1, #data:get() do
                        out[i] = 40
                    end
                    return out
                end,
                w_full = true,
                virtualize = "ancestor",
                row = function(item)
                    return ui.div {{ id = item.id, w_full = true, h = 40 }}
                end,
            }}
        end, {{ w_full = true{enter} }}),
    }},
}}"##
    )
}

/// The rows in view built, of the first `rows`.
fn built(h: &mut Harness, rows: usize) -> Vec<usize> {
    (1..=rows)
        .filter(|i| h.find(&format!("row_{i}")).is_some())
        .collect()
}

#[test]
fn a_list_swapped_in_with_a_fade_builds_its_rows_as_the_fade_starts() {
    let mut h = Harness::inline(&swapped_in(
        ", enter = { opacity = { 0, 1 }, duration = 120 }",
    ));
    h.frames(2);
    h.click("go");
    // Mid-fade: the rows in view (300 points below a 40-point button).
    h.frames(2);
    let rows = built(&mut h, 40);
    assert!(rows.starts_with(&[1, 2, 3, 4, 5, 6]), "built {rows:?}");
    assert!(!rows.contains(&30), "built far past the view: {rows:?}");
    h.frames(20);
    assert!(built(&mut h, 40).starts_with(&[1, 2, 3, 4, 5, 6]));
    assert_eq!(h.error(), None);
}

#[test]
fn a_list_swapped_in_without_a_fade_builds_the_same_rows() {
    let mut h = Harness::inline(&swapped_in(""));
    h.frames(2);
    h.click("go");
    h.frames(2);
    let rows = built(&mut h, 40);
    assert!(rows.starts_with(&[1, 2, 3, 4, 5, 6]), "built {rows:?}");
    assert_eq!(h.error(), None);
}
