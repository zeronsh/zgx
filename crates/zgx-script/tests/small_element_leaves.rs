//! A small leaf element is one scene node while nothing needs a box around
//! it: an svg or a canvas with a size of its own. A padding, border or
//! decoration arriving later (or a canvas's text) splits it, the node staying
//! the one its ref, semantics and hit testing name.

mod common;

use common::Harness;
use zgx::semantics::Role;

/// The nodes from `id`'s node down (it included).
fn nodes_under(h: &mut Harness, id: &str) -> usize {
    let node = h.find(id).unwrap_or_else(|| panic!("no `{id}`"));
    let scene = h.ui.scene.borrow();
    let (mut n, mut stack) = (0, vec![node]);
    while let Some(at) = stack.pop() {
        n += 1;
        stack.extend(scene.children(at).iter().copied());
    }
    n
}

const LEAVES: &str = r#"
return ui.column {
    ui.svg {
        id = "icon",
        code = "<svg xmlns='http://www.w3.org/2000/svg' width='12' height='12'><rect width='12' height='12'/></svg>",
        w = 12,
        h = 12,
    },
    ui.canvas { id = "line", w = 100, h = 10, draw = function(c, w, h) c:rect(0, 0, w, h, "$accent", 0) end },
}
"#;

#[test]
fn a_sized_svg_is_one_node() {
    let mut h = Harness::inline(LEAVES);
    h.settle();
    assert_eq!(nodes_under(&mut h, "icon"), 1);
}

#[test]
fn a_canvas_drawing_no_text_is_one_node() {
    let mut h = Harness::inline(LEAVES);
    h.settle();
    assert_eq!(nodes_under(&mut h, "line"), 1);
}

const SPLITS: &str = r#"
local padded = ui.state("padded", false)
local says = ui.state("says", false)
local open = ui.state("open", false)
local icon = "<svg xmlns='http://www.w3.org/2000/svg' width='12' height='12'><rect width='12' height='12'/></svg>"
return ui.column {
    ui.button { id = "pad", on_click = function() padded:set(true) end, "pad" },
    ui.button { id = "say", on_click = function() says:set(true) end, "say" },
    ui.button { id = "fade", on_click = function() open:set(true) end, "fade" },
    ui.svg { id = "padded", code = icon, w = 12, h = 12, p = function() return if padded:get() then 2 else 0 end },
    ui.canvas {
        id = "says",
        w = 100,
        h = 20,
        draw = function(c, w, h)
            c:rect(0, 0, w, h, "$accent", 0)
            if says:get() then
                c:text("hi", 4, 4)
            end
        end,
    },
    ui.svg { id = "fades", code = icon, w = 12, h = 12, opacity = ui.tween(open, { duration = 100, from = 0.2, to = 1 }) },
}
"#;

/// A padding arriving splits a carried svg: the node with its id becomes
/// the box, and its artwork a child inset by the padding.
#[test]
fn a_padding_arriving_moves_the_artwork_into_a_child() {
    let mut h = Harness::inline(SPLITS);
    h.settle();
    let node = h.find("padded").unwrap();
    assert_eq!(nodes_under(&mut h, "padded"), 1);
    h.click("pad");
    h.frame();
    h.frame();
    assert_eq!(h.find("padded"), Some(node), "the same node keeps its id");
    assert_eq!(nodes_under(&mut h, "padded"), 2);
    let scene = h.ui.scene.borrow();
    let art = scene.children(node)[0];
    assert!(matches!(scene.kind(art), zgx::scene::NodeKind::Svg(_)));
    let (outer, inner) = (scene.bounds(node), scene.bounds(art));
    assert_eq!(
        (
            inner.x - outer.x,
            inner.y - outer.y,
            inner.width,
            inner.height
        ),
        (2., 2., 8., 8.)
    );
}

/// A canvas drawing text puts its labels in the node, so its drawing
/// moves into a child first.
#[test]
fn a_canvas_drawing_text_moves_its_drawing_into_a_child() {
    let mut h = Harness::inline(SPLITS);
    h.settle();
    assert_eq!(nodes_under(&mut h, "says"), 1);
    h.click("say");
    h.frame();
    h.frame();
    let node = h.find("says").unwrap();
    let scene = h.ui.scene.borrow();
    let children = scene.children(node);
    assert!(
        children.len() >= 2,
        "the drawing and its labels: {}",
        children.len()
    );
    assert!(matches!(
        scene.kind(children[0]),
        zgx::scene::NodeKind::Canvas(_)
    ));
}

/// An opacity animation restyles every frame: the svg keeps carrying its
/// artwork, never put back to a box.
#[test]
fn an_opacity_animation_keeps_the_artwork_carried() {
    let mut h = Harness::inline(SPLITS);
    h.settle();
    h.click("fade");
    for _ in 0..8 {
        h.frame();
        assert_eq!(nodes_under(&mut h, "fades"), 1);
        let node = h.find("fades").unwrap();
        assert!(matches!(
            h.ui.scene.borrow().kind(node),
            zgx::scene::NodeKind::Svg(_)
        ));
    }
}

const NAMED: &str = r#"
local padded = ui.state("padded", false)
local resized = ui.state("resized", 0)
local order = ui.state("order", { "a", "b", "c" })
local r = ui.ref()
local icon = "<svg xmlns='http://www.w3.org/2000/svg' width='12' height='12'><rect width='12' height='12'/></svg>"
return ui.column {
    ui.button { id = "pad", on_click = function() padded:set(true) end, "pad" },
    ui.button {
        id = "reverse",
        on_click = function()
            local o = order:peek()
            order:set({ o[3], o[2], o[1] })
        end,
        "reverse",
    },
    ui.svg {
        id = "named",
        label = "Chart",
        code = icon,
        ref = r,
        w = 12,
        h = 12,
        p = function() return if padded:get() then 2 else 0 end,
        on_resize = function() resized:set(resized:peek() + 1) end,
    },
    ui.text { id = "measured", function() return `w={r.width or 0} n={resized:get() > 0}` end },
    ui.column {
        id = "keyed",
        ui.each(order, function(k)
            return ui.svg { id = `row-{k}`, code = icon, w = 12, h = 12 }
        end, { key = function(k) return k end }),
    },
}
"#;

/// The node a carried svg is keeps what names it, before and after a split:
/// its role and name, the hit at its middle, its ref, its `on_resize`.
#[test]
fn a_carried_svg_keeps_its_name_hit_ref_and_resize_across_a_split() {
    let mut h = Harness::inline(NAMED);
    h.settle();
    for split in [false, true] {
        if split {
            h.click("pad");
            h.frame();
            h.frame();
            assert_eq!(nodes_under(&mut h, "named"), 2);
        } else {
            assert_eq!(nodes_under(&mut h, "named"), 1);
        }
        h.ui.prepare_frame();
        let node = h.find("named").unwrap();
        {
            let semantics = h.ui.semantics.borrow();
            let named = semantics.get(node).expect("its semantics");
            assert_eq!((named.role, named.label.as_str()), (Role::Image, "Chart"));
        }
        let scene = h.ui.scene.borrow();
        let b = scene.bounds(node);
        let hit = scene.hit_test(b.x + b.width / 2., b.y + b.height / 2.);
        assert!(
            hit.is_some_and(|hit| hit == node || scene.children(node).contains(&hit)),
            "split {split}: its middle hits it"
        );
        drop(scene);
        let shown = h.text("measured");
        assert!(
            shown.contains("w=12") && shown.contains("n=true"),
            "split {split}: {shown}"
        );
    }
}

/// Keyed svgs, reordered, keep one node each and their order.
#[test]
fn keyed_carried_svgs_reorder_as_one_node_each() {
    let mut h = Harness::inline(NAMED);
    h.settle();
    let before: Vec<_> = ["a", "b", "c"]
        .map(|k| h.find(&format!("row-{k}")).unwrap())
        .into();
    h.click("reverse");
    h.frame();
    let column = h.find("keyed").unwrap();
    let scene = h.ui.scene.borrow();
    let rows: Vec<_> = scene.children(column).to_vec();
    drop(scene);
    for k in ["a", "b", "c"] {
        assert_eq!(nodes_under(&mut h, &format!("row-{k}")), 1);
    }
    let after: Vec<_> = ["c", "b", "a"]
        .map(|k| h.find(&format!("row-{k}")).unwrap())
        .into();
    assert_eq!(
        after,
        before.iter().rev().copied().collect::<Vec<_>>(),
        "the same nodes, moved"
    );
    assert!(rows.windows(2).all(|w| w[0] != w[1]));
}

/// A split in the frame that also scrolls the list holding it: the new
/// child is laid out and placed in that frame (no blank or stale frame).
#[test]
fn a_split_while_its_list_scrolls_shows_its_child_that_frame() {
    let code = r#"
local padded = ui.state("padded", false)
local icon = "<svg xmlns='http://www.w3.org/2000/svg' width='12' height='12'><rect width='12' height='12'/></svg>"
local rows = {}
for i = 1, 40 do
    table.insert(rows, ui.text { `row {i}` })
end
table.insert(rows, ui.svg { id = "deep", code = icon, w = 12, h = 12, p = function() return if padded:get() then 2 else 0 end })
return ui.column {
    ui.button { id = "pad", on_click = function() padded:set(true) end, "pad" },
    ui.scroll { id = "pane", h = 60, ui.column(rows) },
}
"#;
    let mut h = Harness::inline(code);
    h.settle();
    h.click("pad");
    h.scroll("pane", 10_000.);
    h.frame();
    let node = h.find("deep").unwrap();
    let scene = h.ui.scene.borrow();
    assert_eq!(scene.children(node).len(), 1, "split in that frame");
    let (outer, art) = (scene.bounds(node), scene.bounds(scene.children(node)[0]));
    assert_eq!(
        (art.x - outer.x, art.y - outer.y, art.width, art.height),
        (2., 2., 8., 8.),
        "laid out and placed in that frame"
    );
}

/// An svg entering and leaving with a presence is one node while it shows.
#[test]
fn a_carried_svg_enters_and_leaves_as_one_node() {
    let code = r#"
local on = ui.state("on", false)
local icon = "<svg xmlns='http://www.w3.org/2000/svg' width='12' height='12'><rect width='12' height='12'/></svg>"
return ui.column {
    ui.button { id = "toggle", on_click = function() on:set(not on:peek()) end, "toggle" },
    ui.presence(function(): string?
        return if on:get() then "p" else nil
    end, function()
        return ui.svg { id = "entering", code = icon, w = 12, h = 12 }
    end, { enter = { opacity = { 0, 1 }, duration = 100 }, exit = { opacity = { 1, 0 }, duration = 100 } }),
}
"#;
    let mut h = Harness::inline(code);
    h.settle();
    assert!(h.find("entering").is_none());
    h.click("toggle");
    for _ in 0..8 {
        h.frame();
        assert_eq!(nodes_under(&mut h, "entering"), 1);
    }
    h.click("toggle");
    for _ in 0..12 {
        h.frame();
    }
    assert!(h.find("entering").is_none(), "gone once it has left");
}
