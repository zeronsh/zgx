//! Every size takes a theme token as `w` and `gap` do: `gap_x`, `gap_y`,
//! `min_w`, `max_w`, `min_h` and `max_h` given `"$space.1"` are the
//! token's points, as given its number. And `gap_x` or `gap_y` refine
//! `gap` whichever form each is given in (CSS's `column-gap` and `row-gap`
//! over `gap`).
mod common;
use common::Harness;

/// The token's points, as the theme gives them to a script.
fn points(token: &str) -> f32 {
    let mut h = Harness::inline(&format!(
        r#"return ui.text {{ id = "v", tostring(ui.theme.get("{token}")) }}"#
    ));
    h.frames(1);
    h.text("v").parse().unwrap()
}

#[test]
fn an_axis_gap_takes_a_token() {
    let mut h = Harness::inline(
        r#"return ui.column {
            ui.row { id = "x", gap_x = "$space.1",
                ui.div { id = "a", w = 10, h = 10 }, ui.div { id = "b", w = 10, h = 10 } },
            ui.column { id = "y", gap_y = "$space.1",
                ui.div { id = "c", w = 10, h = 10 }, ui.div { id = "d", w = 10, h = 10 } },
        }"#,
    );
    h.frames(2);
    let space = points("space.1");
    assert!(space > 0.);
    let (a, b) = (h.bounds("a"), h.bounds("b"));
    assert_eq!(b.x - (a.x + a.width), space, "gap_x: the token's points");
    let (c, d) = (h.bounds("c"), h.bounds("d"));
    assert_eq!(d.y - (c.y + c.height), space, "gap_y: the token's points");
}

#[test]
fn an_axis_gap_refines_a_tokened_gap_whatever_its_form() {
    let mut h = Harness::inline(
        r#"local box = function(id) return ui.div { id = id, w = 30, h = 10, shrink = 0 } end
        return ui.column {
            ui.row { w = 70, flex_wrap = true, gap = "$space.1", gap_y = 0,
                box("a"), box("b"), box("c") },
            ui.row { gap = "$space.1", gap_x = 10, box("d"), box("e") },
            ui.row { gap = 3, gap_x = "$space.1", box("f"), box("g") },
        }"#,
    );
    h.frames(2);
    let space = points("space.1");
    let (a, b, c) = (h.bounds("a"), h.bounds("b"), h.bounds("c"));
    assert_eq!(b.x - (a.x + a.width), space, "across: the token gap");
    assert_eq!(c.y, a.y + a.height, "between lines: gap_y = 0 wins");
    let (d, e) = (h.bounds("d"), h.bounds("e"));
    assert_eq!(
        e.x - (d.x + d.width),
        10.,
        "a number gap_x over a token gap"
    );
    let (f, g) = (h.bounds("f"), h.bounds("g"));
    assert_eq!(
        g.x - (f.x + f.width),
        space,
        "a token gap_x over a number gap"
    );
}

#[test]
fn min_and_max_sizes_take_tokens() {
    let mut h = Harness::inline(
        r#"return ui.column { items = "start",
            ui.div { id = "min_w", min_w = "$space.4", h = 5 },
            ui.div { id = "max_w", w = 500, max_w = "$space.4", h = 5 },
            ui.div { id = "min_h", w = 5, min_h = "$space.4" },
            ui.div { id = "max_h", w = 5, h = 500, max_h = "$space.4" },
        }"#,
    );
    h.frames(2);
    let space = points("space.4");
    assert!(space > 0.);
    assert_eq!(h.bounds("min_w").width, space);
    assert_eq!(h.bounds("max_w").width, space);
    assert_eq!(h.bounds("min_h").height, space);
    assert_eq!(h.bounds("max_h").height, space);
}
