//! A list that scrolls with the scroll view around it
//! (`virtualize = "ancestor"`): a panel scrolling as a whole, its sections
//! lists at full height. It builds only the rows that view shows (and its
//! overscan), as the view scrolls; it is as tall as all its rows, so the
//! view's extent is right; and Tab, focus and a row asked for reach rows not
//! built.
mod common;
use common::Harness;
use zgx::input::{Key, Modifiers};

/// The rows built, by index, read from their texts (`row {i}`).
fn built(h: &mut Harness) -> Vec<usize> {
    let tree = h.tree();
    let mut rows: Vec<usize> = tree
        .lines()
        .filter_map(|line| {
            let at = line.find("\"row ")? + 5;
            line[at..].split('"').next()?.parse().ok()
        })
        .collect();
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// `app` in a window tall enough for the whole panel.
fn mount(app: &str) -> Harness {
    let h = Harness::inline(app);
    h.ui.scene.borrow_mut().resize(400., 700.);
    h.frames(3);
    h
}

fn panel(list: &str) -> String {
    format!(
        r#"return ui.column {{ w = 300, h = 600,
            ui.button {{ id = "above", w = 80, h = 20, "Above" }},
            ui.scroll {{ id = "panel", w = 300, h = 400,
                ui.column {{ w_full = true,
                    ui.text {{ id = "header", h = 40, "Side chats" }},
                    {list},
                }},
            }},
        }}"#
    )
}

const FIXED: &str = r#"ui.list { id = "chats", count = 1000, height = 20, virtualize = "ancestor",
    w_full = true,
    row = function(i)
        return ui.row { h = 20, ui.button { label = `row {i}`, w = 120, h = 18, `row {i}` } }
    end }"#;

#[test]
fn a_list_in_a_scroll_builds_the_rows_the_scroll_shows() {
    let mut h = mount(&panel(FIXED));
    // 400 points of a scroll, 40 of them its header: about 18 rows of 20,
    // and the overscan.
    let rows = built(&mut h);
    assert!(
        rows.len() <= 400 / 20 + 6 && rows.first() == Some(&1) && rows.contains(&18),
        "{rows:?}"
    );
    // As tall as all its rows: the scroll's extent is right.
    assert_eq!(h.bounds("chats").height, 20_000.);
    // Scrolled 4000 points: the rows from about 199 (40 for the header).
    h.scroll("panel", 4000.);
    h.frames(3);
    let rows = built(&mut h);
    assert!(
        rows.len() <= 400 / 20 + 6 && rows.contains(&199) && rows.contains(&215),
        "{rows:?}"
    );
    assert!(!rows.contains(&1), "the top's rows left: {rows:?}");
}

#[test]
fn a_list_of_row_heights_in_a_scroll_builds_the_rows_the_scroll_shows() {
    // Rows alternately 20 and 40 tall: 30 on average.
    let list = r#"ui.list { id = "chats", virtualize = "ancestor", w_full = true,
        heights = (function()
            local out = {}
            for i = 1, 1000 do out[i] = if i % 2 == 0 then 40 else 20 end
            return out
        end)(),
        row = function(i)
            return ui.row { ui.button { label = `row {i}`, w = 120, h = 18, `row {i}` } }
        end }"#;
    let mut h = mount(&panel(list));
    let rows = built(&mut h);
    assert!(
        rows.len() <= 400 / 20 + 6 && rows.first() == Some(&1),
        "{rows:?}"
    );
    assert_eq!(h.bounds("chats").height, 30_000.);
    // 6000 points down, 5960 into the list: a pair of rows is 60 points,
    // so row 199 (99 pairs on) is at its top.
    h.scroll("panel", 6000.);
    h.frames(3);
    let rows = built(&mut h);
    assert!(rows.contains(&199) && rows.contains(&205), "{rows:?}");
    assert!(rows.len() <= 400 / 20 + 6, "{rows:?}");
}

#[test]
fn two_lists_in_one_scroll_build_only_their_rows_in_view() {
    // A panel's two sections: the second's rows come as it scrolls in.
    let lists = r#"ui.list { id = "first", count = 30, height = 20, virtualize = "ancestor", w_full = true,
            row = function(i) return ui.text { h = 20, `first {i}` } end },
        ui.list { id = "second", count = 500, height = 20, virtualize = "ancestor", w_full = true,
            row = function(i) return ui.button { label = `row {i}`, w = 120, h = 18, `row {i}` } end }"#;
    let mut h = mount(&panel(lists));
    // The first section (600 points) fills the view: none of the second's.
    assert_eq!(built(&mut h), Vec::<usize>::new());
    // 700 points down: the second list starts 640 in (header and first),
    // so 60 points of it are past, its row 4 at the top.
    h.scroll("panel", 700.);
    h.frames(3);
    let rows = built(&mut h);
    assert!(
        rows.contains(&4) && !rows.contains(&1) && rows.len() <= 400 / 20 + 6,
        "{rows:?}"
    );
}

/// Whose accessible name focus is on.
fn focused(h: &mut Harness) -> String {
    let node = h.ui.input.focused().expect("focus");
    h.ui.semantics
        .borrow()
        .get(node)
        .map(|s| s.label.clone())
        .unwrap_or_default()
}

#[test]
fn tab_goes_on_down_a_list_in_a_scroll_past_the_rows_it_built() {
    let mut h = mount(&panel(FIXED));
    h.click("above");
    let mut seen = Vec::new();
    for _ in 0..40 {
        h.key(Key::Tab);
        h.frames(2);
        seen.push(focused(&mut h));
    }
    let want: Vec<String> = (1..=40).map(|i| format!("row {i}")).collect();
    assert_eq!(seen, want);
    // Each one shown: the scroll followed focus down.
    let node = h.ui.input.focused().unwrap();
    let rect = h.ui.scene.borrow().visible_bounds(node);
    assert!(rect.is_some_and(|r| r.height > 0.), "{rect:?}");
    // And back up with Shift+Tab.
    for _ in 0..5 {
        h.key_with(
            Key::Tab,
            Modifiers {
                shift: true,
                ..Default::default()
            },
        );
        h.frames(2);
    }
    assert_eq!(focused(&mut h), "row 35");
}

#[test]
fn a_row_asked_for_scrolls_the_scroll_around_the_list_to_it() {
    let list = r#"ui.list { id = "chats", ref = chats, count = 1000, height = 20,
        virtualize = "ancestor", w_full = true,
        row = function(i)
            return ui.row { h = 20, ui.button { label = `row {i}`, w = 120, h = 18, `row {i}` } }
        end }"#;
    let app = format!(
        r#"local chats = ui.ref()
        ui.command {{ id = "go", run = function() chats:peek():scroll_to_row(700) end }}
        {}"#,
        panel(list)
    );
    let mut h = mount(&app);
    assert!(h.host.run_command("go"));
    h.frames(4);
    let rows = built(&mut h);
    assert!(rows.contains(&700), "{rows:?}");
    let shown = h
        .find("\"row 700\"")
        .and_then(|node| h.ui.scene.borrow().visible_bounds(node));
    assert!(shown.is_some_and(|r| r.height > 0.), "{shown:?}");
}

#[test]
fn a_measured_list_or_another_value_is_an_error() {
    for (list, says) in [
        (
            r#"ui.list { count = 10, measured = true, virtualize = "ancestor",
                row = function(i) return ui.text { `row {i}` } end }"#,
            "not `measured`",
        ),
        (
            r#"ui.list { count = 10, virtualize = "self",
                row = function(i) return ui.text { `row {i}` } end }"#,
            "`virtualize` is \"ancestor\"",
        ),
    ] {
        let error = zgx_script::testing::TestApp::try_new(|runtime| {
            zgx_script::ScriptHost::inline(runtime, "app", format!("return {list}"))
        })
        .err()
        .unwrap_or_default();
        assert!(error.contains(says), "{error}");
    }
}

/// A reload finds focus again on its row, scrolled to with the view around
/// the list.
#[test]
fn a_reload_keeps_focus_on_a_row_of_a_list_in_a_scroll() {
    let dir = std::env::temp_dir().join(format!("zgx-list-in-scroll-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("app.luau");
    std::fs::write(&path, panel(FIXED)).unwrap();
    let mut h = Harness::new(move |runtime| zgx_script::ScriptHost::new(runtime, path.clone()));
    h.ui.scene.borrow_mut().resize(400., 700.);
    h.frames(3);
    h.click("above");
    for _ in 0..30 {
        h.key(Key::Tab);
        h.frames(2);
    }
    assert_eq!(focused(&mut h), "row 30");
    h.host.reload().expect("it reloads");
    h.frames(10);
    assert_eq!(focused(&mut h), "row 30");
}

/// Keyed rows stay the rows they were (not built again) as the view around
/// the list scrolls a little.
#[test]
fn a_keyed_lists_rows_stay_as_the_scroll_around_it_moves() {
    let list = r#"ui.list { id = "chats", data = ui.state("chats", (function()
            local out = {}
            for i = 1, 1000 do out[i] = { id = `c{i}`, n = i } end
            return out
        end)()), key = function(chat) return chat.id end,
        height = 20, virtualize = "ancestor", w_full = true,
        row = function(chat)
            return ui.row { h = 20, ui.button { label = `row {chat.n}`, w = 120, h = 18, `row {chat.n}` } }
        end }"#;
    let mut h = mount(&panel(list));
    let before = h.find("\"row 10\"").expect("row 10 built");
    h.scroll("panel", 40.);
    h.frames(3);
    assert_eq!(
        h.find("\"row 10\""),
        Some(before),
        "the same row, not built again"
    );
    assert!(built(&mut h).contains(&21));
}

/// Zeron's panel section: the list in a clipping box that eases open and
/// shut. The rows it builds follow the box, not only the scroll: folded
/// none, opened again (the scroll not moved) the rows it shows, and the box
/// shrinking keeps only those it still shows.
#[test]
fn a_list_in_a_box_easing_open_builds_the_rows_the_box_shows() {
    let app = r#"local mounted = ui.state("mounted", true)
        local amount = ui.state("amount", 1)
        ui.command { id = "fold", run = function() amount:set(0) end }
        ui.command { id = "unmount", run = function() mounted:set(false) end }
        ui.command { id = "mount", run = function() mounted:set(true) end }
        ui.command { id = "step", run = function() amount:set(math.min(1, amount:peek() + 0.25)) end }
        ui.command { id = "nearly", run = function() amount:set(0.01) end }
        return ui.column { w = 400, h = 700,
            ui.scroll { id = "view", w = 400, h = 600,
                ui.column { w_full = true,
                    ui.button { id = "above", w = 80, h = 20, "Above" },
                    ui.show(mounted, ui.div { overflow = "hidden", w_full = true,
                        h = function() return math.floor(9000 * amount:get()) end,
                        ui.list { id = "chats", count = 300, virtualize = "ancestor", w_full = true,
                            heights = (function()
                                local out = {}
                                for i = 1, 300 do out[i] = 30 end
                                return out
                            end)(),
                            row = function(i)
                                return ui.button { label = `row {i}`, w = 120, h = 28, `row {i}` }
                            end },
                    }),
                },
            },
        }"#;
    let mut h = mount(app);
    let first = built(&mut h);
    assert!(first.len() >= 19 && first.contains(&19), "{first:?}");
    for command in ["fold", "unmount"] {
        assert!(h.host.run_command(command));
        h.frames(3);
    }
    assert_eq!(built(&mut h), Vec::<usize>::new(), "folded");
    assert!(h.host.run_command("mount"));
    h.frames(3);
    for _ in 0..4 {
        assert!(h.host.run_command("step"));
        h.frames(3);
    }
    // Open again, the scroll where it was: the rows it shows.
    let reopened = built(&mut h);
    assert_eq!(reopened, first, "reopened");
    // Nearly shut: 90 points of it, three rows and the overscan.
    assert!(h.host.run_command("nearly"));
    h.frames(3);
    let shut = built(&mut h);
    assert!(
        !shut.is_empty() && shut.len() <= 3 + 2 && shut.first() == Some(&1),
        "{shut:?}"
    );
}
