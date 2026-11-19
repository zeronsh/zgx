//! A `ui.list` with `animate` slides the rows it places elsewhere, as
//! `ui.each`'s `animate` slides its rows: a row growing above them, a row
//! coming in between. Frame by frame, each row of the list is where the same
//! row of a `ui.each` with the same motion is.
mod common;

use common::Harness;

/// Eight rows of 30 points, shown twice: by a `ui.list` (`heights`) and by
/// a `ui.each`, each with `animate`. Row 2 grows to 100 points while
/// `open` is set; `insert` adds a row of 30 after row 2. A `ui.each` slides
/// rows its list moves (`animate`); rows that layout moves (a row above
/// growing) slide with `layout = "position"` (`by_layout`), which a list
/// change would slide a second time.
fn twin(animate: &str, by_layout: bool) -> Harness {
    twin_in(animate, by_layout, 500)
}
/// `twin`, its list `list_height` points tall and building no row it
/// doesn't show (`overscan = 0`).
fn twin_in(animate: &str, by_layout: bool, list_height: u32) -> Harness {
    let layout = match by_layout {
        true => r#"layout = "position", layout_ease = "linear", layout_duration = 200,"#,
        false => "",
    };
    Harness::inline(&format!(
        r##"local open = ui.state("open", false)
local items = {{}}
for i = 1, 8 do
    items[i] = {{ id = "r" .. i }}
end
local data = ui.state("rows", items)
local function tall(item)
    return if item.id == "r2" and open:get() then 100 else 30
end
return ui.row {{ w = 700, h = 600,
    ui.column {{ w = 100,
        ui.button {{ id = "toggle", "Toggle", on_click = function() open:set(not open:get()) end }},
        ui.button {{ id = "insert", "Insert", on_click = function()
            local now = table.clone(data:get())
            table.insert(now, 3, {{ id = "gap" }})
            data:set(now)
        end }},
    }},
    ui.list {{ id = "list", data = data, key = "id", w = 200, h = {list_height}, overscan = 0,
        heights = function()
            local out = {{}}
            for i, item in data:get() do
                out[i] = tall(item)
            end
            return out
        end,
        {animate}
        row = function(item)
            return ui.div {{ id = "l_" .. item.id, w_full = true, h_full = true }}
        end,
    }},
    ui.column {{ id = "each", w = 200, h = 500,
        ui.each(data, function(item)
            return ui.div {{ id = "e_" .. item.id, w_full = true, h = function() return tall(item) end,
                {layout} }}
        end, {{ key = function(item) return item.id end, {animate} }}),
    }},
}}"##
    ))
}

const LINEAR: &str = r#"animate = { duration = 200, ease = "linear" },"#;

/// Where row `id` shows in each, from its container's top: the list's, the
/// `each`'s.
fn places(h: &mut Harness, id: &str) -> (f32, f32) {
    let (list, each) = (h.bounds("list").y, h.bounds("each").y);
    (
        h.bounds(&format!("l_{id}")).y - list,
        h.bounds(&format!("e_{id}")).y - each,
    )
}

/// Rows `ids`, frame by frame for `frames` frames after a click on
/// `button`: the list's place of each against the `each`'s, and every
/// place each row showed in.
fn follow(h: &mut Harness, button: &str, ids: &[&str], frames: usize) -> Vec<Vec<f32>> {
    h.click(button);
    let mut shown = vec![Vec::new(); ids.len()];
    for frame in 0..frames {
        h.frame();
        for (row, id) in ids.iter().enumerate() {
            let (list, each) = places(h, id);
            assert!(
                (list - each).abs() < 0.5,
                "frame {frame}: {id} at {list} in the list, {each} in the each"
            );
            shown[row].push(list);
        }
    }
    shown
}

/// It slid: some frame shows it strictly between where it was and is.
fn slid(places: &[f32], from: f32, to: f32) -> bool {
    let (low, high) = (from.min(to) + 0.5, from.max(to) - 0.5);
    places.iter().any(|y| (low..=high).contains(y))
}

#[test]
fn rows_below_a_row_growing_slide_down_as_each_rows_do_and_back() {
    let mut h = twin(LINEAR, true);
    h.frames(3);
    assert_eq!(places(&mut h, "r3"), (60., 60.));
    let shown = follow(&mut h, "toggle", &["r3", "r5", "r8"], 20);
    assert!(slid(&shown[0], 60., 130.), "r3 jumped: {:?}", shown[0]);
    assert!(slid(&shown[2], 210., 280.), "r8 jumped: {:?}", shown[2]);
    assert_eq!(places(&mut h, "r3"), (130., 130.));
    // And back up as it closes.
    let shown = follow(&mut h, "toggle", &["r3", "r5"], 20);
    assert!(slid(&shown[0], 130., 60.), "r3 jumped back: {:?}", shown[0]);
    assert_eq!(places(&mut h, "r5"), (120., 120.));
    assert_eq!(h.error(), None);
}

#[test]
fn rows_after_a_gap_opening_slide_down_as_each_rows_do() {
    let mut h = twin(LINEAR, false);
    h.frames(3);
    let shown = follow(&mut h, "insert", &["r3", "r6"], 20);
    assert!(slid(&shown[0], 60., 90.), "r3 jumped: {:?}", shown[0]);
    assert!(slid(&shown[1], 150., 180.), "r6 jumped: {:?}", shown[1]);
    assert_eq!(places(&mut h, "r6"), (180., 180.));
    assert_eq!(h.error(), None);
}

#[test]
fn with_reduced_motion_rows_are_placed_at_once() {
    let mut h = twin(LINEAR, true);
    h.frames(3);
    zgx::theme::Themes::of(&h.ui.runtime).override_reduced_motion(Some(true));
    h.click("toggle");
    h.frame();
    assert_eq!(places(&mut h, "r5").0, 190.);
    assert_eq!(h.error(), None);
}

#[test]
fn a_list_without_animate_wraps_its_rows_in_nothing() {
    let depth = |animate: &str| {
        let mut h = twin(animate, false);
        h.frames(2);
        let row = h.find("l_r1").expect("a row");
        let list = h.find("list").expect("the list");
        let scene = h.ui.scene.borrow();
        scene.ancestors(row).take_while(|a| *a != list).count()
    };
    // The list places its row node; the slide moves a box inside it.
    assert_eq!(depth(LINEAR), depth("") + 1);
}

#[test]
fn a_row_coming_into_view_as_rows_close_up_slides_in_from_where_it_was() {
    // 150 points of list show five rows of 30; with row 2 open (100), the
    // rows r1 to r3 alone (r4 starts at 160).
    let mut h = twin_in(LINEAR, true, 150);
    h.click("toggle");
    h.frames(20);
    assert!(h.find("l_r4").is_none(), "r4 is out of view, not built");
    assert!(h.find("l_r5").is_none(), "r5 is out of view, not built");
    // Closing row 2 brings r4 and r5 into view: built now, each slides up
    // from where it would have been (160, 190), as the each's rows do.
    let shown = follow(&mut h, "toggle", &["r4", "r5"], 20);
    assert!(slid(&shown[0], 160., 90.), "r4 jumped: {:?}", shown[0]);
    assert!(slid(&shown[1], 190., 120.), "r5 jumped: {:?}", shown[1]);
    assert_eq!(places(&mut h, "r5"), (120., 120.));
    assert_eq!(h.error(), None);
}

#[test]
fn rows_a_row_growing_pushes_out_of_view_slide_out_before_they_go() {
    // 150 points of list show r1 to r5 (r5 at 120); opening row 2 (to
    // 100) pushes r4 and r5 out of view, to 160 and 190.
    let mut h = twin_in(LINEAR, true, 150);
    h.frames(3);
    assert!(h.find("l_r5").is_some(), "r5 shows");
    // They stay while they slide, as the each's same rows do.
    let shown = follow(&mut h, "toggle", &["r4", "r5"], 10);
    assert!(slid(&shown[1], 120., 190.), "r5 jumped: {:?}", shown[1]);
    // Then they go.
    h.frames(20);
    assert!(h.find("l_r4").is_none(), "r4 released");
    assert!(h.find("l_r5").is_none(), "r5 released");
    assert_eq!(h.error(), None);
}
