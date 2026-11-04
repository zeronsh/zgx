//! What takes the pointer around `pointer_events = "none"`: nothing but a
//! descendant with handlers of its own, and no box a `ui.show` adds.
mod common;

use common::Harness;

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
