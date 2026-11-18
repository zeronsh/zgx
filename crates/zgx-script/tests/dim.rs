//! `dim`: whether a disabled control looks disabled. A control is dimmed
//! while disabled unless it styles itself; `dim = false` keeps it as it is,
//! and a state or function decides as it changes, the node not built again
//! and still disabled to assistive technology (a sending button keeps its
//! ink).
mod common;
use common::Harness;
use zgx_script::Value;

fn opacity(h: &mut Harness, id: &str) -> f32 {
    let node = h.find(id).unwrap_or_else(|| panic!("no node `{id}`"));
    h.ui.scene.borrow().effects(node).opacity
}
fn disabled(h: &mut Harness, id: &str) -> bool {
    let node = h.find(id).unwrap();
    h.ui.semantics
        .borrow()
        .get(node)
        .is_some_and(|s| s.disabled)
}

#[test]
fn a_function_dims_a_disabled_control_as_it_changes() {
    let mut h = Harness::inline(
        r#"local sending = ui.state("sending", false)
        return ui.column {
            ui.button { id = "send", disabled = true,
                dim = function() return not sending:get() end, "Send" },
        }"#,
    );
    h.frames(2);
    let node = h.find("send");
    assert!((opacity(&mut h, "send") - 0.45).abs() < 1e-6, "dimmed");
    h.host.set_state("sending", Value::Bool(true)).unwrap();
    h.frames(2);
    assert_eq!(opacity(&mut h, "send"), 1., "sending: its ink kept");
    assert!(disabled(&mut h, "send"), "still disabled");
    assert_eq!(h.find("send"), node, "the same node: not built again");
    h.host.set_state("sending", Value::Bool(false)).unwrap();
    h.frames(2);
    assert!(
        (opacity(&mut h, "send") - 0.45).abs() < 1e-6,
        "dimmed again"
    );
    assert_eq!(h.host.error().get(), None);
}

#[test]
fn a_state_dims_as_it_changes() {
    let mut h = Harness::inline(
        r#"local dim = ui.state("dim", true)
        return ui.button { id = "b", disabled = true, dim = dim, "B" }"#,
    );
    h.frames(2);
    assert!((opacity(&mut h, "b") - 0.45).abs() < 1e-6);
    h.host.set_state("dim", Value::Bool(false)).unwrap();
    h.frames(2);
    assert_eq!(opacity(&mut h, "b"), 1.);
}

#[test]
fn dim_false_keeps_a_disabled_control_as_it_is() {
    let mut h =
        Harness::inline(r#"return ui.button { id = "b", disabled = true, dim = false, "B" }"#);
    h.frames(2);
    assert_eq!(opacity(&mut h, "b"), 1.);
    assert!(disabled(&mut h, "b"));
}

#[test]
fn what_gives_no_dim_is_as_before() {
    let mut h = Harness::inline(
        r#"return ui.column {
            ui.button { id = "control", disabled = true, "C" },
            ui.button { id = "enabled", "E" },
            ui.div { id = "box", disabled = true, w = 10, h = 10 },
        }"#,
    );
    h.frames(2);
    assert!(
        (opacity(&mut h, "control") - 0.45).abs() < 1e-6,
        "a disabled control dims"
    );
    assert_eq!(opacity(&mut h, "enabled"), 1.);
    assert_eq!(opacity(&mut h, "box"), 1., "a box has no dim of its own");
}

#[test]
fn dim_decides_over_a_control_s_own_disabled_style() {
    // Its own disabled style, from its variants, is what dimmed is: no 0.45
    // over it, even when it sets no opacity; not dimmed, it keeps its ink.
    let mut h = Harness::inline(
        r##"local dim = ui.state("dim", true)
        local faded = ui.variants { base = { disabled = { opacity = 0.6 } } }
        local inked = ui.variants { base = { disabled = { bg = "#202020" } } }
        return ui.column {
            ui.button { id = "a", disabled = true, dim = dim, variants = faded, "A" },
            ui.button { id = "b", disabled = true, dim = false, variants = faded, "B" },
            ui.button { id = "c", disabled = true, dim = dim, variants = inked, "C" },
        }"##,
    );
    h.frames(2);
    assert!(
        (opacity(&mut h, "a") - 0.6).abs() < 1e-6,
        "its own disabled style"
    );
    assert_eq!(opacity(&mut h, "b"), 1., "dim = false over its own");
    assert_eq!(opacity(&mut h, "c"), 1., "its own style, no dim over it");
    h.host.set_state("dim", Value::Bool(false)).unwrap();
    h.frames(2);
    assert_eq!(opacity(&mut h, "a"), 1., "not dimmed");
    assert!(disabled(&mut h, "a"), "still disabled");
    h.host.set_state("dim", Value::Bool(true)).unwrap();
    h.frames(2);
    assert!((opacity(&mut h, "a") - 0.6).abs() < 1e-6, "dimmed again");
    assert_eq!(h.host.error().get(), None);
}

fn fill(h: &mut Harness, id: &str) -> zgx::scene::Color {
    let node = h.find(id).unwrap();
    match h.ui.scene.borrow().kind(node) {
        zgx::scene::NodeKind::Panel { quad, .. } => quad.fill,
        other => panic!("not a panel: {other:?}"),
    }
}

#[test]
fn dim_and_state_chosen_variants_follow_each_on_its_own() {
    let mut h = Harness::inline(
        r##"local tone = ui.state("tone", "red")
        local dim = ui.state("dim", true)
        local box = ui.variants {
            variants = { tone = { red = { bg = "#ff0000" }, blue = { bg = "#0000ff" } } },
            defaults = { tone = "red" },
        }
        return ui.div { id = "box", w = 10, h = 10, disabled = true, dim = dim,
            variants = box { tone = tone } }"##,
    );
    h.frames(2);
    let node = h.find("box");
    assert_eq!(fill(&mut h, "box"), zgx::scene::Color(255, 0, 0, 255));
    assert!((opacity(&mut h, "box") - 0.45).abs() < 1e-6, "dimmed");
    h.host.set_state("tone", Value::from("blue")).unwrap();
    h.frames(2);
    assert_eq!(
        fill(&mut h, "box"),
        zgx::scene::Color(0, 0, 255, 255),
        "the variant followed"
    );
    assert!((opacity(&mut h, "box") - 0.45).abs() < 1e-6, "still dimmed");
    h.host.set_state("dim", Value::Bool(false)).unwrap();
    h.frames(2);
    assert_eq!(opacity(&mut h, "box"), 1., "not dimmed");
    assert_eq!(
        fill(&mut h, "box"),
        zgx::scene::Color(0, 0, 255, 255),
        "its variant kept"
    );
    assert_eq!(h.find("box"), node, "the same node");
    assert_eq!(h.host.error().get(), None);
}
