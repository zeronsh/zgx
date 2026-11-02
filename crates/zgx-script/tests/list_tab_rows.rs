//! Tab and Shift+Tab go on through a list's rows, built or not: past the
//! rows a list has built, the next row is shown, built and its control
//! focused, as a page's Tab goes on down a list it holds whole, rather
//! than leaving the list (or wrapping round) halfway.
mod common;
use common::Harness;
use zgx::input::{Key, Modifiers};

const APP: &str = r#"
local rows = {}
for i = 1, 60 do
    rows[i] = { id = i, n = 2 + (i * 7) % 23 }
end
return ui.column { w = 600, h = 700,
    ui.row { id = "header", focusable = true, h = 30, w_full = true, ui.text { "header" } },
    ui.list { id = "list", data = ui.state("rows", rows), measured = true, height = 40, overscan = 0,
        grow = true, w_full = true,
        key = function(r) return r.id end,
        row = function(r)
            local lines = {}
            for k = 1, r.n do lines[k] = ui.text { `line {k} of reply {r.id}` } end
            return ui.column { p = 8, gap = 4, ui.column(lines),
                ui.button { label = `tool {r.id}`, w = 120, h = 24, `tool {r.id}` } }
        end },
    ui.row { id = "composer", focusable = true, h = 80, w_full = true, ui.text { "composer" } },
}
"#;

/// What each of `n` presses of Tab (or Shift+Tab, `back`) focuses, by
/// accessible name.
fn walk(h: &mut Harness, back: bool, n: usize) -> Vec<String> {
    (0..n)
        .map(|_| {
            h.key_with(
                Key::Tab,
                Modifiers {
                    shift: back,
                    ..Default::default()
                },
            );
            h.frames(5);
            let node = h.ui.input.focused().expect("focus");
            h.ui.semantics
                .borrow()
                .get(node)
                .map(|s| s.label.clone())
                .unwrap_or_default()
        })
        .collect()
}

#[test]
fn tab_goes_on_down_a_list_past_the_rows_it_built() {
    let mut h = Harness::inline(APP);
    h.ui.scene.borrow_mut().resize(600., 700.);
    h.frames(10);
    h.click("header");
    let seen = walk(&mut h, false, 20);
    let want: Vec<String> = (1..=20).map(|i| format!("tool {i}")).collect();
    assert_eq!(seen, want);
}

#[test]
fn shift_tab_goes_on_up_a_list_at_its_end_past_the_rows_it_built() {
    let mut h = Harness::inline(APP);
    h.ui.scene.borrow_mut().resize(600., 700.);
    h.frames(10);
    // To the end, as a transcript opens: the list measures its rows as it
    // goes, so a few scrolls.
    for _ in 0..5 {
        h.scroll("list", 100_000.);
        h.frames(10);
    }
    h.click("composer");
    let seen = walk(&mut h, true, 20);
    let want: Vec<String> = (41..=60).rev().map(|i| format!("tool {i}")).collect();
    assert_eq!(seen, want);
}

/// Shift+Tab from below a list shown at its top enters at its last row,
/// not the last it built, as it would enter a list held whole.
#[test]
fn shift_tab_into_a_list_from_below_enters_at_its_last_row() {
    let mut h = Harness::inline(APP);
    h.ui.scene.borrow_mut().resize(600., 700.);
    h.frames(10);
    h.click("composer");
    let seen = walk(&mut h, true, 4);
    let want: Vec<String> = (57..=60).rev().map(|i| format!("tool {i}")).collect();
    assert_eq!(seen, want);
}

/// And Tab from above a list shown at its end enters at its first row.
#[test]
fn tab_into_a_list_from_above_enters_at_its_first_row() {
    let mut h = Harness::inline(APP);
    h.ui.scene.borrow_mut().resize(600., 700.);
    h.frames(10);
    for _ in 0..5 {
        h.scroll("list", 100_000.);
        h.frames(10);
    }
    h.click("header");
    let seen = walk(&mut h, false, 4);
    let want: Vec<String> = (1..=4).map(|i| format!("tool {i}")).collect();
    assert_eq!(seen, want);
}
