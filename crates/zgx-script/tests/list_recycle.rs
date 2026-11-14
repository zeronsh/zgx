//! A list with `recycle = true` writes the rows it scrolls away with the
//! items coming into view instead of building rows for them: a jump past
//! its view shows the new items in the rows it had, and builds none.
mod common;
use common::Harness;

fn app(recycle: bool) -> String {
    format!(
        r#"local top = ui.state("top", 0)
        local builds = ui.state("builds", 0)
        local items = {{}}
        for i = 1, 1000 do items[i] = {{ id = `k{{i}}`, name = `item {{i}}` }} end
        local data = ui.state("items", items)
        local built = 0
        return ui.column {{
            ui.command {{ id = "jump", run = function() top:set(top:peek() + 400) end }},
            ui.command {{ id = "back", run = function() top:set(0) end }},
            ui.command {{ id = "rename", run = function()
                local now = table.clone(data:peek())
                now[1] = {{ id = "k1", name = "renamed" }}
                data:set(now)
            end }},
            ui.command {{ id = "count", run = function() builds:set(built) end }},
            ui.text {{ function() return `builds {{builds:get()}}` end }},
            ui.list {{ data = data, key = function(item) return item.id end, height = 20, h = 100, w = 200,
                overscan = 0, recycle = {recycle}, offset = top,
                row = function(_item, _i, live)
                    built += 1
                    return ui.text {{ function() return live:get().name end, h = 20 }}
                end }},
        }}"#
    )
}

fn builds(h: &mut Harness) -> usize {
    assert!(h.host.run_command("count"));
    h.frames(1);
    let tree = h.tree();
    let at = tree.find("builds ").expect("the count") + 7;
    tree[at..].split('"').next().unwrap().parse().unwrap()
}

fn jump(h: &mut Harness, times: usize) {
    for _ in 0..times {
        assert!(h.host.run_command("jump"));
        h.frames(1);
    }
}

#[test]
fn a_recycling_list_shows_new_items_in_the_rows_it_had() {
    let mut h = Harness::inline(&app(true));
    h.frames(2);
    let first = builds(&mut h);
    assert!(first <= 6, "{first}");
    jump(&mut h, 30);
    // 30 × 400 points: rows 600 to 605 show, written into the first rows.
    let tree = h.tree();
    assert!(tree.contains("\"item 601\""), "{tree}");
    assert!(tree.contains("\"item 605\""), "{tree}");
    assert!(!tree.contains("\"item 1\""), "{tree}");
    let after = builds(&mut h);
    assert!(after <= first + 1, "built {after} rows after {first}");
    // Back at the top, the first items again, and a change to one shows.
    assert!(h.host.run_command("back"));
    h.frames(1);
    assert!(h.tree().contains("\"item 1\""));
    assert!(h.host.run_command("rename"));
    h.frames(1);
    let tree = h.tree();
    assert!(tree.contains("\"renamed\""), "{tree}");
    assert!(tree.contains("\"item 2\""), "{tree}");
}

#[test]
fn a_list_without_recycle_builds_the_rows_it_scrolls_to() {
    let mut h = Harness::inline(&app(false));
    h.frames(2);
    let first = builds(&mut h);
    jump(&mut h, 30);
    assert!(h.tree().contains("\"item 601\""));
    assert!(builds(&mut h) >= first + 30 * 5);
}

#[test]
fn a_recycling_list_with_spare_time_shows_the_right_items() {
    let mut h = Harness(zgx_script::testing::TestApp::with_spare_time(|runtime| {
        zgx_script::ScriptHost::inline(runtime, "app", app(true))
    }));
    h.frames(2);
    // Warm: what it builds ahead and keeps, once.
    for _ in 0..3 {
        assert!(h.host.run_command("jump"));
        h.frames(1);
        h.spend_spare_time();
        h.frames(1);
    }
    let warm = builds(&mut h);
    for _ in 0..27 {
        assert!(h.host.run_command("jump"));
        h.frames(1);
        h.spend_spare_time();
        h.frames(1);
    }
    // Rows scrolled away wait hidden for the next: no more are built.
    let after = builds(&mut h);
    assert!(after <= warm + 2, "built {after} after {warm}");
    let tree = h.tree();
    for i in 601..=605 {
        assert!(tree.contains(&format!("\"item {i}\"")), "{i}: {tree}");
    }
    // Each row shows its own item once: none repeated, none stale.
    let mut shown: Vec<&str> = tree.lines().filter(|l| l.contains("\"item ")).collect();
    let before = shown.len();
    shown.sort_unstable();
    shown.dedup();
    assert_eq!(before, shown.len(), "{tree}");
}
