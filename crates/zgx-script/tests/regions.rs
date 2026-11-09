//! Control-flow regions (`show`, `switch`, `presence`) and `layout` boxes
//! are invisible to layout: what they hold sizes and places itself as if it
//! stood in their place, at once.
mod common;

use common::Harness;
use zgx::scene::Rect;

impl Harness {
    fn rect(&mut self, id: &str) -> Rect {
        let node = self.find(id).unwrap_or_else(|| panic!("no node `{id}`"));
        self.ui.scene.borrow().bounds(node)
    }
    fn run_frames(&mut self, n: usize) {
        for _ in 0..n {
            self.frame();
        }
    }
}

/// The editor body under `show` fills the window's height (its `grow`
/// reaches the column it stands in), and what replaces it sizes itself.
#[test]
fn an_editor_body_under_show_fills_the_window() {
    let mut h = Harness::inline(
        r##"local editing = ui.state("editing", true)
        return ui.column { w_full = true, h_full = true, items = "stretch",
            ui.row { id = "toolbar", h = 40 },
            ui.show(editing,
                ui.column { id = "body", grow = 1, items = "stretch",
                    ui.textarea { id = "area", label = "Body", value = ui.state("text", ""), h_full = true } },
                ui.text { "No note", id = "none" }),
            ui.row { id = "status", h = 20 },
            ui.button { id = "toggle", w = 10, h = 10, absolute = true, on_click = function() editing:set(not editing:get()) end },
        }"##,
    );
    assert_eq!(h.rect("body"), Rect::new(0., 40., 400., 240.));
    assert_eq!(
        h.rect("area").height,
        240.,
        "its percentage height resolves"
    );
    assert_eq!(h.rect("status").y, 280.);
    h.click("toggle");
    let none = h.rect("none");
    assert!(none.y == 40. && none.height < 40., "{none:?}");
    assert_eq!(h.rect("status").y, 40. + none.height);
    h.click("toggle");
    assert_eq!(h.rect("body"), Rect::new(0., 40., 400., 240.));
}

/// What `show` and `switch` hold takes percentages, `self` and `absolute`
/// insets against the real parent, and neither grows in a row.
#[test]
fn show_and_switch_lay_out_as_their_content() {
    let mut h = Harness::inline(
        r##"local tab = ui.state("tab", "a")
        return ui.overlay { w = 400, h = 300,
            ui.row { w_full = true, h = 100, items = "start",
                ui.show(true, ui.div { id = "half", w = "50%", h = 10 }),
                ui.switch(tab, {
                    a = ui.div { id = "a", w = 30, h = 30, self = "end" },
                    b = ui.div { id = "b", grow = 1, h = 20 },
                }),
                ui.div { id = "after", w = 10, h = 10 },
            },
            ui.show(true, ui.column { id = "sheet", absolute = true, top = 10, left = 20, right = 0, bottom = 0 }),
            ui.button { id = "go", w = 1, h = 1, absolute = true, on_click = ui.set(tab, "b") },
        }"##,
    );
    assert_eq!(h.rect("half"), Rect::new(0., 0., 200., 10.));
    assert_eq!(
        h.rect("a"),
        Rect::new(200., 70., 30., 30.),
        "`self` reaches the row"
    );
    assert_eq!(h.rect("after").x, 230., "nothing grows that did not ask to");
    assert_eq!(
        h.rect("sheet"),
        Rect::new(20., 10., 380., 290.),
        "insets against the overlay"
    );
    h.click("go");
    assert_eq!(
        h.rect("b"),
        Rect::new(200., 0., 190., 20.),
        "the new case's `grow`"
    );
    assert_eq!(h.rect("after").x, 390.);
}

/// `presence` with sizing of its own is a box its content fills; without,
/// its content lays out in its place. What leaves stays where it was, as
/// large as it was, out of the flow.
#[test]
fn presence_applies_its_sizing_and_its_content_fills_it() {
    let mut h = Harness::inline(
        r##"local tab = ui.state("tab", "a")
        return ui.row { w_full = true, h_full = true, items = "start",
            ui.presence(tab, function(v)
                return ui.column { id = "pane-" .. v, min_w = "10%", ui.text { v } }
            end, { w = 120, h_full = true, exit = { opacity = 0, duration = 100 } }),
            ui.div { id = "rest", grow = 1, h = 10 },
            ui.presence(tab, function(v) return ui.div { id = "tag-" .. v, w = 30, h = 8 } end),
            ui.button { id = "go", w = 1, h = 1, absolute = true, on_click = ui.set(tab, "b") },
        }"##,
    );
    assert_eq!(h.rect("pane-a"), Rect::new(0., 0., 120., 300.));
    assert_eq!(h.rect("rest"), Rect::new(120., 0., 250., 10.));
    assert_eq!(
        h.rect("tag-a"),
        Rect::new(370., 0., 30., 8.),
        "as if in its place"
    );
    h.click("go");
    h.frame();
    assert_eq!(
        h.rect("pane-b"),
        Rect::new(0., 0., 120., 300.),
        "entering fills it"
    );
    assert_eq!(
        h.rect("pane-a"),
        Rect::new(0., 0., 120., 300.),
        "leaving keeps its place"
    );
    assert_eq!(h.rect("rest"), Rect::new(120., 0., 250., 10.));
    h.run_frames(10);
    assert!(h.find("pane-a").is_none(), "gone after its exit");
    assert_eq!(h.rect("pane-b"), Rect::new(0., 0., 120., 300.));
}

/// A presence's box lays its content out as a one-cell grid with stretched
/// items would, the presence's own `items` and `justify` included (a grid
/// reads `items` down, and `justify` moves no track that fills the box).
#[test]
fn a_presence_box_lays_out_as_a_one_cell_grid() {
    let content = [
        r#"ui.div { id = "content", w = 40, h = 20 }"#,
        r#"ui.text { id = "content", "a few words that wrap in the box", wrap = true }"#,
    ];
    let props = [
        "w = 120, h = 100",
        "w = 120, h = 100, items = 'center'",
        "w = 120, h = 100, justify = 'center'",
        "w = 120, h = 100, items = 'end', justify = 'end'",
        "w = 120, p = 10",
        "grow = 1, py = 4",
    ];
    for content in content {
        for props in props {
            let items = if props.contains("items") {
                ""
            } else {
                "items = 'stretch', "
            };
            let presence =
                format!("ui.presence(tab, function() return {content} end, {{ {props} }})");
            let grid = format!(
                "ui.column {{ grid = true, cols = 1, rows = 1, {items}{props}, {content} }}"
            );
            let rects: Vec<_> = [presence, grid]
                .iter()
                .map(|boxed| {
                    let mut h = Harness::inline(&format!(
                        r#"local tab = ui.state("tab", "a")
                        return ui.column {{ w_full = true, h_full = true, items = "start", {boxed} }}"#
                    ));
                    h.rect("content")
                })
                .collect();
            assert_eq!(rects[0], rects[1], "{props}: {content}");
        }
    }
}

/// However it sets its width (or doesn't), a presence's box lays its content
/// out and answers the room it has as a one-cell grid with stretched items
/// does: its content wrapped or not, aligning itself, with margins or
/// absolute, in a parent that sizes it, one sized by what it holds, and a
/// row too narrow for it. A layout other than that grid has to hold here
/// (an overlay differs where the width is left to the content, and moves a
/// row's siblings by leaving taffy).
#[test]
fn a_presence_box_follows_its_room_as_a_one_cell_grid_does() {
    let props = [
        "p = 10",
        "h = 60",
        "w = 120, p = 10",
        "w = width, p = 10",
        "w = '50%', p = 10",
        "w_full = true, p = 10",
    ];
    let contents = [
        r#"ui.div { id = "content", w = 40, h = 20 }"#,
        r#"ui.div { id = "content", w = 40, h = 20, self = "center" }"#,
        r#"ui.div { id = "content", m = 8, h = 20, self = "center" }"#,
        r#"ui.div { id = "content", absolute = true, top = 5, left = 7, w = 40, h = 20 }"#,
        r#"ui.text { id = "content", "a few words that wrap in the box" }"#,
        r#"ui.text { id = "content", "a few words that wrap in the box", wrap = true }"#,
        r#"ui.text { id = "content", "a few words that wrap in the box", wrap = true, self = "end" }"#,
        r#"ui.text { id = "content", "a verylongunbreakablewordthatsetstheline ok", wrap = true }"#,
        r#"ui.text { id = "content", "a few words that wrap in the box", wrap = true, mx = 5 }"#,
    ];
    let parents = [
        ("ui.column { w = room, items = 'start', %, }", ""),
        (
            "ui.column { items = 'start', ui.column { items = 'start', %, } }",
            "",
        ),
        (
            "ui.row { w = room, ui.div { id = 'beside', w = 90, h = 10 }, %, }",
            "beside",
        ),
    ];
    let mut wrong = Vec::new();
    for content in contents {
        for props in props {
            for (parent, beside) in parents {
                let rects = [
                    format!("ui.presence(tab, function() return {content} end, {{ {props} }})"),
                    format!(
                        "ui.column {{ grid = true, cols = 1, rows = 1, items = 'stretch', {props}, {content} }}"
                    ),
                ]
                .map(|boxed| {
                    let parent = parent.replace('%', &boxed);
                    let mut h = Harness::inline(&format!(
                        r#"local tab = ui.state("tab", "a")
                        local room = ui.state("room", 300)
                        local width = ui.state("width", 120)
                        return ui.column {{ w_full = true, h_full = true, items = "start",
                            {parent},
                            ui.button {{ id = "narrow", w = 4, h = 4, on_click = ui.set(room, 90) }},
                            ui.button {{ id = "wide", w = 4, h = 4, on_click = ui.set(room, 300) }},
                            ui.button {{ id = "wider", w = 4, h = 4, on_click = ui.set(width, 160) }},
                        }}"#
                    ));
                    let mut shown = Vec::new();
                    for button in ["", "narrow", "wide", "wider"] {
                        if !button.is_empty() {
                            h.click(button);
                        }
                        h.run_frames(2);
                        shown.push(h.rect("content"));
                        if !beside.is_empty() {
                            shown.push(h.rect(beside));
                        }
                    }
                    shown
                });
                if rects[0] != rects[1] {
                    wrong.push(format!(
                        "{props} in {parent}: {content}\n  presence {:?}\n  grid     {:?}",
                        rects[0], rects[1]
                    ));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// `size` is a box's width and height at once, a number or a pair, literal or
/// followed, on a presence too (which it makes a box its content fills). On
/// what is no box it does nothing, as it did everywhere.
#[test]
fn size_sizes_a_box_and_nothing_else() {
    for element in ["ui.div", "ui.column", "ui.row", "ui.overlay", "ui.scroll"] {
        for (size, width, height) in [("50", 50., 50.), ("{ 120, 60 }", 120., 60.)] {
            let mut h = Harness::inline(&format!(
                r#"return ui.column {{ w_full = true, h_full = true, items = "start",
                    {element} {{ id = "box", size = {size} }},
                }}"#
            ));
            h.run_frames(2);
            let rect = h.rect("box");
            assert_eq!(
                (rect.width, rect.height),
                (width, height),
                "{element}: {size}"
            );
        }
    }
    for (size, width, height) in [("50", 50., 50.), ("{ 120, 60 }", 120., 60.)] {
        let mut h = Harness::inline(&format!(
            r#"local tab = ui.state("tab", "a")
            return ui.column {{ w_full = true, h_full = true, items = "start",
                ui.presence(tab, function()
                    return ui.div {{ id = "content" }}
                end, {{ size = {size} }}),
            }}"#
        ));
        h.run_frames(2);
        assert_eq!(
            h.rect("content"),
            Rect::new(0., 0., width, height),
            "{size}"
        );
    }
    let mut h = Harness::inline(
        r#"local size = ui.state("size", { 120, 60 })
        return ui.column { w_full = true, h_full = true, items = "start",
            ui.div { id = "box", size = size },
            ui.button { id = "grow", w = 4, h = 4, on_click = ui.set(size, { 200, 100 }) },
        }"#,
    );
    h.run_frames(2);
    assert_eq!(h.rect("box"), Rect::new(0., 0., 120., 60.));
    h.click("grow");
    h.run_frames(2);
    assert_eq!(h.rect("box"), Rect::new(0., 0., 200., 100.));
    // On a text and a button it is as if it weren't there, and so is a word
    // a component's `size = "sm"` hands along to the box it builds.
    for (with, without) in [
        (
            r#"ui.div { id = "it", size = "sm", w = 40, h = 20 }"#,
            r#"ui.div { id = "it", w = 40, h = 20 }"#,
        ),
        (
            r#"ui.text { id = "it", "words", size = 14 }"#,
            r#"ui.text { id = "it", "words" }"#,
        ),
        (
            r#"ui.button { id = "it", "Send", size = 24 }"#,
            r#"ui.button { id = "it", "Send" }"#,
        ),
    ] {
        let rects = [with, without].map(|element| {
            let mut h = Harness::inline(&format!(
                r#"return ui.column {{ w_full = true, h_full = true, items = "start", {element} }}"#
            ));
            h.run_frames(2);
            h.rect("it")
        });
        assert_eq!(rects[0], rects[1], "{with}");
    }
}

/// `mode = "wait"`: while the old content leaves, nothing takes its place
/// and it stays where it was.
#[test]
fn waiting_presence_keeps_what_leaves_in_place() {
    let mut h = Harness::inline(
        r##"local tab = ui.state("tab", "a")
        return ui.column { items = "start",
            ui.div { h = 50, w = 10 },
            ui.presence(tab, function(v) return ui.div { id = v, w = 40, h = 30 } end,
                { mode = "wait", exit = { opacity = 0, duration = 100 } }),
            ui.div { id = "below", w = 10, h = 10 },
            ui.button { id = "go", w = 1, h = 1, absolute = true, on_click = ui.set(tab, "b") },
        }"##,
    );
    assert_eq!(h.rect("a"), Rect::new(0., 50., 40., 30.));
    assert_eq!(h.rect("below").y, 80.);
    h.click("go");
    h.frame();
    assert_eq!(
        h.rect("a"),
        Rect::new(0., 50., 40., 30.),
        "leaving, in place"
    );
    assert_eq!(h.rect("below").y, 50., "out of the flow");
    h.run_frames(12);
    assert!(h.find("a").is_none());
    assert_eq!(h.rect("b"), Rect::new(0., 50., 40., 30.));
    assert_eq!(h.rect("below").y, 80.);
}

/// A `layout = "width"` pane (the notes app's preview) is at its full
/// size at once, the scroll view inside it too; only the space it takes
/// eases open.
#[test]
fn a_width_layout_box_lays_its_content_out_at_the_target_size() {
    let mut h = Harness::inline(
        r##"local open = ui.state("open", false)
        local rows = {}
        for i = 1, 40 do rows[i] = ui.text { "row " .. i } end
        return ui.row { w_full = true, h_full = true, items = "stretch",
            ui.column { id = "main", grow = 1 },
            ui.column { id = "pane", layout = "width", layout_ease = "linear", layout_duration = 100,
                w = function() return open:get() and 160 or 0 end,
                h_full = true, shrink = 0, overflow = "hidden", items = "stretch",
                ui.scroll { id = "scroll", grow = 1, w_full = true,
                    ui.column { id = "list", w_full = true, rows } } },
            ui.button { id = "go", w = 1, h = 1, absolute = true, on_click = function() open:set(not open:get()) end },
        }"##,
    );
    h.run_frames(30);
    assert_eq!(h.rect("main").width, 400.);
    h.click("go");
    h.ui.prepare_frame();
    h.settle();
    let main = h.rect("main");
    assert_eq!(main.width, 400., "no jump on the frame it opens");
    let scroll = h.rect("scroll");
    assert_eq!(
        (scroll.width, scroll.height),
        (160., 300.),
        "the scroll view at once"
    );
    assert_eq!(h.rect("list").width, 160.);
    h.frame();
    h.run_frames(3);
    let mid = h.rect("main").width;
    assert!(mid < 390. && mid > 250., "easing open: {mid}");
    let scroll = h.rect("scroll");
    assert_eq!(
        (scroll.width, scroll.height),
        (160., 300.),
        "still full size"
    );
    h.run_frames(8);
    assert_eq!(h.rect("main").width, 240.);
    assert_eq!(h.rect("pane"), Rect::new(240., 0., 160., 300.));
    assert!(!h.clock.wants_frame());
}

/// `layout = "height"` on a node that grows: its `grow` and percentages
/// reach its parent as if the box were not there.
#[test]
fn a_layout_box_keeps_its_nodes_flex() {
    let mut h = Harness::inline(
        r##"return ui.column { w_full = true, h_full = true, items = "stretch",
            ui.div { h = 20 },
            ui.column { id = "grows", layout = "height", grow = 1, mx = 10, items = "stretch",
                ui.div { id = "fill", h = "50%" } },
            ui.div { id = "last", h = 20 },
        }"##,
    );
    assert_eq!(h.rect("grows"), Rect::new(10., 20., 380., 260.));
    assert_eq!(h.rect("fill").height, 130.);
    assert_eq!(h.rect("last").y, 280.);
}

/// A layout box clipping (`layout_clip`) cuts its content to the space it
/// takes while it eases.
#[test]
fn a_clipping_layout_box_clips_to_the_space() {
    let mut h = Harness::inline(
        r##"local tall = ui.state("tall", false)
        return ui.column {
            ui.column { id = "box", layout = "height", layout_clip = true, layout_ease = "linear", layout_duration = 100,
                ui.div { w = 50, h = function() return tall:get() and 100 or 20 end } },
            ui.button { id = "go", w = 1, h = 1, absolute = true, on_click = ui.set(tall, true) },
        }"##,
    );
    h.click("go");
    h.frame();
    h.run_frames(3);
    let node = h.find("box").unwrap();
    let scene = h.ui.scene.borrow();
    let wrapper = scene.parent(node).unwrap();
    let inset = scene.clip_inset(wrapper).expect("a clip while easing");
    let bottom = match inset.bottom {
        zgx::layout::Length::Px(v) => v,
        other => panic!("{other:?}"),
    };
    assert!(
        bottom > 5. && bottom < 75.,
        "the unshown part is cut: {bottom}"
    );
}

/// A layout box inside a layout box (a fold in a group that folds too): the
/// inner one eases its change, and the outer one's space follows it frame
/// for frame, so what follows the outer box stays right under the content,
/// never overlapping its foot or cutting it. A change of the outer box's own
/// (a row arriving) still eases as it did.
#[test]
fn a_layout_box_follows_one_easing_inside_it() {
    let mut h = Harness::inline(
        r##"local tall = ui.state("tall", false)
        local more = ui.state("more", false)
        return ui.column {
            ui.column { id = "outer", layout = "height", layout_clip = true, layout_ease = "outQuart", layout_duration = 200,
                ui.column { id = "inner", layout = "height", layout_clip = true, layout_ease = "outQuart", layout_duration = 260,
                    ui.div { w = 50, h = function() return tall:get() and 140 or 20 end } },
                ui.div { id = "foot", w = 50, h = 10 },
                ui.show(more, ui.div { w = 50, h = 60 }) },
            ui.div { id = "after", w = 50, h = 10 },
            ui.button { id = "go", w = 1, h = 1, absolute = true, left = 300, top = 0, on_click = function() tall:set(not tall:get()) end },
            ui.button { id = "add", w = 1, h = 1, absolute = true, left = 340, top = 0, on_click = function() more:set(not more:get()) end },
        }"##,
    );
    h.run_frames(5);
    let rest = h.rect("after").y - (h.rect("foot").y + 10.);
    assert!(rest.abs() < 0.5, "at rest, right under it: {rest}");
    for way in ["open", "fold"] {
        h.click("go");
        let mut moved = false;
        for frame in 1..=24 {
            h.frame();
            let (foot, after) = (h.rect("foot"), h.rect("after"));
            let gap = after.y - (foot.y + foot.height);
            assert!(
                gap.abs() < 0.5,
                "{way}, frame {frame}: what follows is {gap} from the content's foot"
            );
            moved |= frame > 1 && foot.y > 25. && foot.y < 155.;
        }
        assert!(moved, "{way}: the inner box eased");
        h.run_frames(10);
    }
    // The outer box's own change (a row arriving in it) eases as ever.
    h.click("add");
    h.frame();
    h.run_frames(3);
    let after = h.rect("after").y;
    let foot = h.rect("foot");
    let full = foot.y + foot.height + 60.;
    assert!(
        after < full - 5.,
        "its own change eases: {after} short of {full}"
    );
    h.run_frames(20);
    assert!((h.rect("after").y - full).abs() < 0.5);
}

/// A fold in a fold, with a row the outer one may take (`more`).
const NESTED_FOLDS: &str = r##"local tall = ui.state("tall", false)
    local more = ui.state("more", false)
    return ui.column {
        ui.column { id = "outer", layout = "height", layout_clip = true, layout_ease = "outQuart", layout_duration = 200,
            ui.column { id = "inner", layout = "height", layout_clip = true, layout_ease = "outQuart", layout_duration = 260,
                ui.div { w = 50, h = function() return tall:get() and 140 or 20 end } },
            ui.div { id = "foot", w = 50, h = 10 },
            ui.show(more, ui.div { w = 50, h = 60 }) },
        ui.div { id = "after", w = 50, h = 10 },
        ui.button { id = "go", w = 1, h = 1, absolute = true, left = 300, top = 0, on_click = function() tall:set(not tall:get()) end },
        ui.button { id = "add", w = 1, h = 1, absolute = true, left = 340, top = 0, on_click = function() more:set(not more:get()) end },
    }"##;

/// Two windows (on one thread, their nodes numbered alike): a box easing in
/// one is never taken for one inside a box of the other. The other's own
/// change still eases while it does.
#[test]
fn a_layout_box_follows_only_boxes_in_its_own_window() {
    let mut a = Harness::inline(NESTED_FOLDS);
    let mut b = Harness::inline(NESTED_FOLDS);
    a.run_frames(5);
    b.run_frames(5);
    // The other window's inner fold eases as this one's outer box takes a row.
    b.click("go");
    a.click("add");
    for _ in 0..3 {
        b.frame();
        a.frame();
    }
    let after = a.rect("after").y;
    let foot = a.rect("foot");
    let full = foot.y + foot.height + 60.;
    assert!(
        after < full - 5.,
        "its own change eases, whatever the other window does: {after} short of {full}"
    );
    a.run_frames(20);
    assert!((a.rect("after").y - full).abs() < 0.5);
}

/// A page of size-animated boxes mounting (a long transcript's folds): no
/// box looks through the others on its first size; a change later looks once
/// for each box it reaches.
#[test]
fn layout_boxes_mounting_look_for_none_inside_them() {
    let before = zgx_script::layout_box_scans();
    let mut h = Harness::inline(
        r##"local tall = ui.state("tall", false)
        local folds = {}
        for i = 1, 400 do
            table.insert(folds, ui.column { layout = "height", layout_clip = true,
                ui.column { layout = "height", layout_clip = true,
                    ui.div { w = 20, h = function() return (i == 1 and tall:get()) and 30 or 10 end } } })
        end
        table.insert(folds, ui.button { id = "go", w = 1, h = 1, absolute = true, left = 300, top = 0, on_click = function() tall:set(not tall:get()) end })
        return ui.column(folds)"##,
    );
    h.run_frames(5);
    let mounted = zgx_script::layout_box_scans() - before;
    assert_eq!(
        mounted, 0,
        "800 boxes mounting looked through the others {mounted} times"
    );
    h.click("go");
    h.run_frames(3);
    let changed = zgx_script::layout_box_scans() - before;
    assert!(
        changed > 0 && changed < 20,
        "one fold's change looks a few times, not for every box: {changed}"
    );
}

#[test]
fn a_presence_box_showing_nothing_lets_the_pointer_through() {
    use zgx::input::{InputEvent, PointerButton};
    let mut h = Harness::inline(
        r#"
local open = ui.state("open", false)
local dropped = ui.state("dropped", "")
return ui.row {
    w_full = true,
    h_full = true,
    items = "stretch",
    -- Window keys: the root listens, under the box.
    on_key = function() return false end,
    ui.column {
        grow = 1,
        items = "stretch",
        ui.div { id = "source", h = 40, on_drag = function() return "card" end },
        ui.div {
            id = "target",
            grow = 1,
            drop_when = function() return true end,
            on_drop = function(payload) dropped:set(payload) end,
        },
        ui.text { "{dropped}", id = "dropped" },
    },
    -- A sheet over everything, put away.
    ui.presence(function() return if open:get() then "sheet" else nil end, function()
        return ui.div { w_full = true, h_full = true }
    end, { absolute = true, top = 0, left = 0, right = 0, bottom = 0 }),
}
"#,
    );
    let (from, to) = (h.bounds("source"), h.bounds("target"));
    let (x, y) = (from.x + from.width / 2., from.y + from.height / 2.);
    let (tx, ty) = (to.x + to.width / 2., to.y + to.height / 2.);
    h.ui.dispatch(InputEvent::PointerMove { x, y });
    h.ui.dispatch(InputEvent::PointerDown {
        x,
        y,
        button: PointerButton::Primary,
    });
    for step in 1..=8 {
        let t = step as f32 / 8.;
        h.ui.dispatch(InputEvent::PointerMove {
            x: x + (tx - x) * t,
            y: y + (ty - y) * t,
        });
    }
    h.ui.dispatch(InputEvent::PointerUp {
        x: tx,
        y: ty,
        button: PointerButton::Primary,
    });
    h.settle();
    assert_eq!(h.text("dropped"), "card", "the empty box is not in the way");
}

#[test]
fn a_presence_box_whose_content_is_hidden_lets_the_pointer_through() {
    // A page kept mounted once built, in a presence box over a button:
    // while the page is hidden or invisible, presses reach the
    // button; shown, they are the page's.
    for hide in ["hidden", "invisible"] {
        let mut h = Harness::inline(&format!(
            r#"
local shown = ui.state("shown", false)
local presses = ui.state("presses", 0)
local built = false
return ui.overlay {{
    w_full = true,
    h_full = true,
    -- The window hears presses (one anywhere closes a menu), under the box.
    on_press = function() end,
    ui.column {{
        ui.button {{ id = "under", w = 80, h = 40,
            on_click = function() presses:update(function(n) return n + 1 end) end }},
        ui.button {{ id = "toggle", w = 80, h = 40,
            on_click = function() shown:update(function(v) return not v end) end }},
        ui.text {{ "{{presses}}", id = "presses" }},
    }},
    ui.presence(function()
        built = built or shown:get()
        return if built then "page" else nil
    end, function()
        return ui.column {{ id = "page", w_full = true, h_full = true,
            {hide} = function() return not shown:get() end }}
    end, {{ absolute = true, top = 0, left = 0, w = 80, h = 40 }}),
}}
"#
        ));
        h.click("under");
        assert_eq!(h.text("presses"), "1", "{hide}: nothing built yet");
        h.click("toggle");
        h.click("under");
        assert_eq!(h.text("presses"), "1", "{hide}: the page shown takes it");
        h.click("toggle");
        h.click("under");
        assert_eq!(
            h.text("presses"),
            "2",
            "{hide}: the page hidden lets it through"
        );
        assert_eq!(h.error(), None);
    }
}

/// A list as tall as its parent builds its rows through what lays out in its
/// place (`show`, `switch`, `presence`, a presence's box) as it does directly;
/// `each` is a box of its own, as its rows lay out in, so given a height.
#[test]
fn a_list_as_tall_as_its_parent_builds_its_rows_through_a_region() {
    let list = r#"ui.list { count = 100, height = 40, h_full = true, w_full = true, row = function(i)
        return ui.div { id = "row-" .. tostring(i), h = 40 }
    end }"#;
    for region in [
        list.to_string(),
        format!("ui.show(true, {list})"),
        format!("ui.switch(tab, {{ a = {list} }})"),
        format!("ui.presence(tab, function() return {list} end)"),
        format!(
            "ui.presence(tab, function() return {list} end, {{ w_full = true, h_full = true }})"
        ),
        format!(
            "ui.each(items, function() return {list} end, {{ key = function(k) return k end, h_full = true, w_full = true }})"
        ),
    ] {
        let mut h = Harness::inline(&format!(
            r#"local tab = ui.state("tab", "a")
            local items = ui.state("items", {{ "one" }})
            return ui.column {{ w_full = true, h_full = true, {region} }}"#
        ));
        h.run_frames(2);
        assert!(h.find("row-1").is_some(), "{region}");
    }
}
