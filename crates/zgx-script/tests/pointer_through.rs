//! What takes the pointer around `pointer_events = "none"`: nothing but a
//! descendant with handlers of its own, and no box a `ui.show` adds.
mod common;

use common::Harness;
use zgx::input::InputEvent;

const FILL: &str = "absolute = true, left = 0, right = 0, top = 0, bottom = 0";

/// `over`, laid over a button filling the same box.
fn over(over: &str) -> Harness {
    Harness::inline(&format!(
        r##"local on = ui.state("on", true)
        local ids = ui.state("ids", {{ "a" }})
        return ui.overlay {{ w = 200, h = 100, on_key = function() return false end,
            ui.button {{ id = "under", w = 200, h = 100, on_click = function() end, "under" }},
            {over}
        }}"##
    ))
}
/// Whether a press in the middle would reach `id`, which must exist.
fn reaches(h: &mut Harness, id: &str) -> bool {
    h.settle();
    let at = {
        let scene = h.ui.scene.borrow();
        h.ui.input.pointer_target(&scene, 100., 50.)
    };
    at == Some(h.find(id).unwrap_or_else(|| panic!("no node `{id}`")))
}

fn layer(holds: &str) -> String {
    format!(r##"ui.div {{ {FILL}, pointer_events = "none", {holds} }}"##)
}

#[test]
fn a_region_shows_a_layer_as_if_in_its_place() {
    let layer = layer(r##"ui.div { w_full = true, h_full = true }"##);
    for (name, region) in [
        ("show", format!("ui.show(true, {layer})")),
        ("show, a state", format!("ui.show(on, {layer})")),
        (
            "show, a function",
            format!("ui.show(function() return on:get() end, {layer})"),
        ),
        (
            "show, its otherwise",
            format!("ui.show(false, ui.div {{}}, {layer})"),
        ),
        (
            "switch",
            format!(r#"ui.switch(function() return "a" end, {{ a = {layer} }})"#),
        ),
        (
            "presence",
            format!("ui.presence(function() return 1 end, function() return {layer} end)"),
        ),
        (
            "show in a show",
            format!("ui.show(true, ui.show(on, {layer}))"),
        ),
    ] {
        let mut h = over(&region);
        assert!(reaches(&mut h, "under"), "{name}: the press passes through");
    }
}

/// The region isn't the only way past: nor is what it holds, nor the row
/// of a list in it.
#[test]
fn a_region_inside_a_layer_takes_nothing() {
    let inner = layer(r##"ui.show(on, ui.div { w_full = true, h_full = true })"##);
    assert!(reaches(&mut over(&inner), "under"));
    let rows = layer(
        r##"ui.each(ids, function() return ui.div { w_full = true, h_full = true } end, {
            key = function(id) return id end, w_full = true, h_full = true, relative = true })"##,
    );
    assert!(reaches(&mut over(&format!("ui.show(on, {rows})")), "under"));
}

/// Scripts find and watch a node without it listening for anything.
#[test]
fn being_found_or_watched_takes_no_pointer() {
    for props in [
        r#"id = "it""#,
        r#"class = "it""#,
        r#"ref = ui.ref()"#,
        "on_mount = function() end",
        "on_resize = function() end",
        "on_visible = function() end",
    ] {
        let code = layer(&format!(
            "ui.div {{ {props}, w_full = true, h_full = true }}"
        ));
        let mut h = over(&code);
        assert_eq!(h.error(), None, "{props}");
        assert!(reaches(&mut h, "under"), "{props}: not its own handlers");
    }
}

#[test]
fn a_descendant_with_handlers_of_its_own_still_takes_it() {
    for handler in [
        "on_click = function() end",
        "on_pointer_move = function() end",
        "hover = { opacity = 0.5 }",
    ] {
        let code = layer(&format!(
            r#"ui.div {{ id = "own", {handler}, w_full = true, h_full = true }}"#
        ));
        let mut h = over(&code);
        assert!(reaches(&mut h, "own"), "{handler}");
    }
    let code = format!(
        "ui.show(true, {})",
        layer(r#"ui.button { id = "own", w = 200, h = 100, on_click = function() end, "x" }"#)
    );
    assert!(reaches(&mut over(&code), "own"), "through a region too");
}

/// A box is still a box: what is laid over without letting the pointer
/// through takes it, through a region as well.
#[test]
fn a_layer_that_lets_nothing_through_still_covers_what_is_under() {
    for code in [
        format!(r#"ui.div {{ {FILL} }}"#),
        format!(r#"ui.show(true, ui.div {{ {FILL} }})"#),
    ] {
        assert!(!reaches(&mut over(&code), "under"), "{code}");
    }
}

/// A contents node listening itself keeps what it was asked to hear.
#[test]
fn a_contents_node_with_a_handler_takes_it() {
    let code = format!(
        r#"ui.div {{ id = "own", {FILL}, display = "contents", on_click = function() end,
            ui.div {{ {FILL}, pointer_events = "none" }} }}"#
    );
    assert!(reaches(&mut over(&code), "own"));
}

/// A draggable's listener comes when it does: the node takes the pointer
/// from then on, and only it.
#[test]
fn a_draggable_takes_the_pointer_once_it_is_made() {
    let mut h = Harness::inline(&format!(
        r##"local made = ui.shared("made", false)
        ui.effect(function()
            if made:get() then anime.createDraggable("#it") end
        end)
        return ui.overlay {{ w = 200, h = 100, on_key = function() return false end,
            ui.button {{ id = "under", w = 200, h = 100, on_click = function() end, "under" }},
            {},
        }}"##,
        layer(r#"ui.div { id = "it", w_full = true, h_full = true }"#)
    ));
    assert!(reaches(&mut h, "under"));
    h.host
        .shared("made", false)
        .set(zgx_script::Value::Bool(true));
    h.frames(3);
    assert_eq!(h.error(), None);
    assert!(reaches(&mut h, "it"));
}

/// `src`, dragged by a press at its middle and moves past the threshold, one
/// frame each; what the script logged.
fn drag_src(code: &str) -> String {
    let mut h = Harness::inline(code);
    h.ui.scene.borrow_mut().resize(400., 300.);
    h.frames(3);
    let b = {
        let node = h.find("press").unwrap();
        h.ui.scene.borrow().bounds(node)
    };
    let (x, y) = (b.x + b.width / 2., b.y + b.height / 2.);
    let button = zgx::input::PointerButton::Primary;
    h.ui.dispatch(InputEvent::PointerMove { x, y });
    h.ui.dispatch(InputEvent::PointerDown { x, y, button });
    h.frames(1);
    for step in 1..6 {
        let (dx, dy) = (step as f32 * 10., step as f32 * 5.);
        h.ui.dispatch(InputEvent::PointerMove {
            x: x + dx,
            y: y + dy,
        });
        h.frames(1);
    }
    h.ui.dispatch(InputEvent::PointerUp {
        x: x + 50.,
        y: y + 25.,
        button,
    });
    h.frames(1);
    h.text("log")
}
/// The same press let go where it was: a click.
fn click_src(code: &str) -> String {
    let mut h = Harness::inline(code);
    h.ui.scene.borrow_mut().resize(400., 300.);
    h.frames(3);
    h.click("press");
    h.frames(1);
    h.text("log")
}
const DRAG_APP: &str = r##"
local log = ui.state("log", "")
local function note(s) log:set(log:peek() .. s .. ";") end
local function source(child, w)
    return ui.div {
        w = w, h = 40,
        on_drag = function() note("drag"); return { kind = "row" } end,
        child,
    }
end
local function button(w, h)
    return ui.div {
        id = "press", w = w, h = h,
        on_click = function() note("click") end,
        hover = { bg = "#222222" },
        ui.text { "Row" },
    }
end
return ui.column { w_full = true, h_full = true,
    ui.text { id = "log", "{log}" },
    BODY,
}
"##;
fn app(body: &str) -> String {
    DRAG_APP.replace("BODY", body)
}

/// A row that's all button is dragged by that button, and still clicks.
#[test]
fn a_drag_starts_from_a_clickable_that_fills_its_source() {
    let code = app("source(button(200, 40), 200)");
    assert_eq!(drag_src(&code), "drag;");
    assert_eq!(click_src(&code), "click;");
}

/// With its padding or a tree's indent around it, as a row has.
#[test]
fn a_drag_starts_from_a_clickable_covering_most_of_its_source() {
    let code = app(r#"ui.div { w = 200, h = 40, px = 8, py = 2,
            on_drag = function() note("drag"); return { kind = "row" } end,
            button(184, 36) }"#);
    assert_eq!(drag_src(&code), "drag;");
    assert_eq!(click_src(&code), "click;");
}

/// As it is a row of a list, the sidebar's and a transcript's shape.
#[test]
fn a_drag_starts_from_a_clickable_row_in_a_list() {
    let each = app(
        r#"ui.each(ui.state("rows", { 1 }), function(r) return source(button(200, 40), 200) end)"#,
    );
    assert_eq!(drag_src(&each), "drag;");
    assert_eq!(click_src(&each), "click;");
    let list = app(
        r#"ui.list { w_full = true, h = 200, data = ui.state("rows", { 1 }),
            key = function(r) return r end,
            row = function(r) return source(button(200, 40), 200) end }"#,
    );
    assert_eq!(drag_src(&list), "drag;");
    assert_eq!(click_src(&list), "click;");
}

/// A control inside a draggable keeps its press: a tab's close button doesn't
/// drag the tab. It goes on being clicked.
#[test]
fn a_small_control_keeps_its_press_from_a_drag_around_it() {
    let code = app("source(button(40, 20), 200)");
    assert_eq!(drag_src(&code), "");
    assert_eq!(click_src(&code), "click;");
}

/// Pressed at `press`'s middle, moved by each of `moves` in turn, and let go
/// at the last.
fn gesture(code: &str, moves: &[(f32, f32)]) -> String {
    let mut h = Harness::inline(code);
    h.ui.scene.borrow_mut().resize(400., 300.);
    h.frames(3);
    let b = {
        let node = h.find("press").unwrap();
        h.ui.scene.borrow().bounds(node)
    };
    let (x, y) = (b.x + b.width / 2., b.y + b.height / 2.);
    let button = zgx::input::PointerButton::Primary;
    h.ui.dispatch(InputEvent::PointerMove { x, y });
    h.ui.dispatch(InputEvent::PointerDown { x, y, button });
    h.frames(1);
    let mut at = (x, y);
    for (dx, dy) in moves {
        at = (x + dx, y + dy);
        h.ui.dispatch(InputEvent::PointerMove { x: at.0, y: at.1 });
        h.frames(1);
    }
    h.ui.dispatch(InputEvent::PointerUp {
        x: at.0,
        y: at.1,
        button,
    });
    h.frames(1);
    h.text("log")
}
const JIGGLE: [(f32, f32); 2] = [(2., 1.), (-1., 1.)];
const MOVED: [(f32, f32); 2] = [(16., 8.), (40., 20.)];
const AND_BACK: [(f32, f32); 3] = [(16., 8.), (40., 20.), (0., 0.)];

/// A hand shakes: a button most of a draggable still clicks under a shake too
/// small to start a drag. Moved, it drags and doesn't click, even let go where
/// it began.
#[test]
fn a_clickable_filling_a_source_clicks_under_a_shake_and_drags_when_moved() {
    for button in ["button(200, 40)", "button(180, 40)"] {
        let code = app(&format!("source({button}, 200)"));
        assert_eq!(gesture(&code, &[]), "click;", "{button}, no movement");
        assert_eq!(gesture(&code, &JIGGLE), "click;", "{button}, a shake");
        assert_eq!(gesture(&code, &MOVED), "drag;", "{button}, moved");
        assert_eq!(gesture(&code, &AND_BACK), "drag;", "{button}, and back");
    }
}

/// Under three quarters of it, a clickable keeps its press however it moves:
/// a click where it's let go on it.
#[test]
fn a_clickable_under_most_of_a_source_never_drags_it() {
    let code = app("source(button(100, 40), 200)");
    assert_eq!(gesture(&code, &JIGGLE), "click;");
    assert_eq!(
        gesture(&code, &[(16., 8.), (20., 10.)]),
        "click;",
        "let go on it"
    );
    assert_eq!(gesture(&code, &AND_BACK), "click;");
    assert_eq!(gesture(&code, &[(200., 8.)]), "", "let go off it");
}

/// Nothing clickable: the drag's own, as ever.
#[test]
fn a_drag_starts_from_what_only_shows_inside_it() {
    let code = app(r#"source(ui.div { id = "press", w = 200, h = 40, ui.text { "Row" } }, 200)"#);
    assert_eq!(drag_src(&code), "drag;");
}

const KEYED_DROP_APP: &str = r##"
local rows = ui.state("rows", { "a", "b", "c" })
local log = ui.state("log", "")
local function row(id)
    local count = ui.use_state(0)
    return ui.div {
        w = 200, h = 40,
        on_drag = function() return { id = id } end,
        PREVIEW
        on_drop = function(p)
            local order = {}
            for _, r in rows:peek() do
                if r ~= p.id then table.insert(order, r) end
            end
            table.insert(order, (table.find(order, id) or #order) + 1, p.id)
            rows:set(order)
            log:set(table.concat(order, ","))
        end,
        ui.div {
            id = "press-" .. id, w = 200, h = 40,
            on_click = function() count:set(count:peek() + 1) end,
            ui.text { id = "count-" .. id, function() return tostring(count:get()) end },
        },
    }
end
return ui.column { w_full = true, h_full = true,
    ui.text { id = "log", "{log}" },
    LIST,
}
"##;

/// `from`'s middle pressed, moved to `to`'s in steps, let go there; what
/// `from` was found as while held.
fn drag_onto(h: &mut Harness, from: &str, to: &str) -> Option<zgx::scene::NodeId> {
    let centre = |h: &mut Harness, id: &str| {
        let node = h.find(id).unwrap();
        let b = h.ui.scene.borrow().bounds(node);
        (b.x + b.width / 2., b.y + b.height / 2.)
    };
    let ((x, y), (tx, ty)) = (centre(h, from), centre(h, to));
    let button = zgx::input::PointerButton::Primary;
    h.ui.dispatch(InputEvent::PointerMove { x, y });
    h.ui.dispatch(InputEvent::PointerDown { x, y, button });
    h.frames(1);
    for step in 1..=8 {
        let t = step as f32 / 8.;
        h.ui.dispatch(InputEvent::PointerMove {
            x: x + (tx - x) * t,
            y: y + (ty - y) * t,
        });
        h.frames(1);
    }
    let held = h.find(from);
    h.ui.dispatch(InputEvent::PointerUp {
        x: tx,
        y: ty,
        button,
    });
    h.frames(2);
    held
}

/// A row dragged and dropped in a keyed list is the same row after: its
/// node, and the state it holds.
#[test]
fn a_dropped_row_keeps_its_node_and_state() {
    let preview = r#"drag_preview = function(p) return ui.div { w = 200, h = 40, ui.text { "moving" } } end,"#;
    for (list, preview) in [
        (r#"ui.each(rows, function(id) return row(id) end)"#, ""),
        (
            r#"ui.list { w_full = true, h = 200, data = rows, height = 40,
            key = function(id) return id end, row = function(id) return row(id) end }"#,
            "",
        ),
        (r#"ui.each(rows, function(id) return row(id) end)"#, preview),
        (
            r#"ui.list { w_full = true, h = 200, data = rows, height = 40,
            key = function(id) return id end, row = function(id) return row(id) end }"#,
            preview,
        ),
    ] {
        let code = KEYED_DROP_APP
            .replace("LIST", list)
            .replace("PREVIEW", preview);
        let mut h = Harness::inline(&code);
        h.ui.scene.borrow_mut().resize(400., 300.);
        h.frames(3);
        assert_eq!(h.error(), None);
        h.click("press-a");
        h.click("press-a");
        h.frames(1);
        assert_eq!(h.text("count-a"), "2", "{list}");
        let node = h.find("press-a");
        let held = drag_onto(&mut h, "press-a", "press-c");
        assert_eq!(h.error(), None);
        assert_eq!(held, node, "{list} {preview}: the same node while held");
        assert_eq!(h.text("log"), "b,c,a", "{list}: dropped after c");
        assert_eq!(h.find("press-a"), node, "{list} {preview}: the same node");
        assert_eq!(h.text("count-a"), "2", "{list} {preview}: its state kept");
    }
}

/// As a virtualized list with its rows' heights given, the held row easing to
/// no height and back once dropped: still the same row, with its state.
#[test]
fn a_held_row_at_no_height_keeps_its_node_and_state() {
    let code = r##"
local rows = ui.state("rows", { "a", "b", "c" })
local held = ui.state("held", "")
local log = ui.state("log", "")
local function row(id)
    local count = ui.use_state(0)
    return ui.div {
        w = 200, h_full = true,
        on_drag = function() held:set(id); return { id = id } end,
        on_drag_end = function() held:set("") end,
        on_drop = function(p)
            local order = {}
            for _, r in rows:peek() do
                if r ~= p.id then table.insert(order, r) end
            end
            table.insert(order, (table.find(order, id) or #order) + 1, p.id)
            rows:set(order)
            held:set("")
            log:set(table.concat(order, ","))
        end,
        ui.div {
            id = "press-" .. id, w = 200, h = 40,
            on_click = function() count:set(count:peek() + 1) end,
            ui.text { id = "count-" .. id, function() return tostring(count:get()) end },
        },
    }
end
return ui.column { w_full = true, h_full = true,
    ui.text { id = "log", "{log}" },
    ui.list { w_full = true, h = 200, data = rows,
        heights = function()
            local hs = {}
            for i, r in rows:get() do hs[i] = if r == held:get() then 0 else 40 end
            return hs
        end,
        key = function(id) return id end, row = function(id) return row(id) end },
}
"##;
    let mut h = Harness::inline(code);
    h.ui.scene.borrow_mut().resize(400., 300.);
    h.frames(3);
    assert_eq!(h.error(), None);
    h.click("press-a");
    h.click("press-a");
    h.frames(1);
    assert_eq!(h.text("count-a"), "2");
    let node = h.find("press-a");
    // Pressed on a, dragged down past the threshold: a goes to no height.
    let (x, y) = {
        let b = h.ui.scene.borrow().bounds(node.unwrap());
        (b.x + b.width / 2., b.y + b.height / 2.)
    };
    let button = zgx::input::PointerButton::Primary;
    h.ui.dispatch(InputEvent::PointerMove { x, y });
    h.ui.dispatch(InputEvent::PointerDown { x, y, button });
    h.frames(1);
    for step in 1..=4 {
        h.ui.dispatch(InputEvent::PointerMove {
            x,
            y: y + step as f32 * 5.,
        });
        h.frames(1);
    }
    assert_eq!(h.text("log"), "");
    let held = h.find("press-a");
    assert_eq!(held, node, "the same node while held at no height");
    // Over c (now in a's place and below), let go.
    let c = h.find("press-c").unwrap();
    let cb = h.ui.scene.borrow().bounds(c);
    let (cx, cy) = (cb.x + cb.width / 2., cb.y + cb.height / 2.);
    h.ui.dispatch(InputEvent::PointerMove { x: cx, y: cy });
    h.frames(1);
    h.ui.dispatch(InputEvent::PointerUp {
        x: cx,
        y: cy,
        button,
    });
    h.frames(3);
    assert_eq!(h.error(), None);
    assert_eq!(h.text("log"), "b,c,a", "dropped after c");
    assert_eq!(held, node, "the same node while held at no height");
    assert_eq!(h.find("press-a"), node, "the same node after");
    assert_eq!(h.text("count-a"), "2", "its state kept");
}
