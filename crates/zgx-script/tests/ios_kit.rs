//! `ui.ios`'s pieces as assistive technology meets them, against UIKit's.
mod common;

use common::Harness;
use zgx::semantics::{Role, SemanticNode};

/// Every semantic node with `role` assistive technology meets, in tree
/// order: none inside a hidden one (`aria_hidden`), as the tree leaves out.
fn with_role(h: &mut Harness, role: Role) -> Vec<SemanticNode> {
    h.ui.prepare_frame();
    let scene = h.ui.scene.borrow();
    let semantics = h.ui.semantics.borrow();
    let mut out = Vec::new();
    let mut pending = vec![scene.root()];
    while let Some(id) = pending.pop() {
        let node = semantics.get(id);
        if node.is_some_and(|n| n.hidden) {
            continue;
        }
        if let Some(node) = node.filter(|n| n.role == role) {
            out.push(node.clone());
        }
        pending.extend(scene.children(id).iter().rev().copied());
    }
    out
}

#[test]
fn a_row_reads_as_a_settings_cell_and_a_toggle_row_is_one_switch() {
    let mut h = Harness::inline(
        r#"local ios = ui.ios
        local on = ui.state("on", false)
        return ios.section {
            header = "Network",
            ios.row { title = "Wi-Fi", detail = "Home", icon = "wifi" },
            ios.row { title = "Airplane Mode", toggle = on, icon = "airplane" },
        }"#,
    );
    h.ui.set_assistive_technology(true);
    h.frames(2);
    assert_eq!(h.error(), None);
    // UIKit's cell: the title its label, the detail its value.
    let buttons = with_role(&mut h, Role::Button);
    let wifi = buttons
        .iter()
        .find(|n| n.label == "Wi-Fi")
        .expect("the Wi-Fi row");
    assert_eq!(wifi.value.as_deref(), Some("Home"));
    // With a switch, one element: the row, its state the switch's.
    let switches = with_role(&mut h, Role::Switch);
    assert_eq!(
        switches.len(),
        1,
        "one switch, not the row and its switch: {switches:?}"
    );
    assert_eq!(switches[0].label, "Airplane Mode");
    assert_eq!(switches[0].checked, Some(false));
    // Its header is a heading, as a table section's is (the headings rotor).
    let headings = with_role(&mut h, Role::Heading);
    assert!(
        headings.iter().any(|n| n.label == "Network"),
        "{headings:?}"
    );
}

impl Harness {
    /// Runs frames at 60 Hz until `ms` have passed.
    fn advance_ms(&mut self, ms: f64) {
        for _ in 0..(ms * 60. / 1000.).round() as usize {
            self.frame();
        }
    }
}

/// Whether node `id` is laid out: neither it nor what holds it is hidden.
fn displayed(h: &mut Harness, id: &str) -> bool {
    let node = h.find(id).unwrap_or_else(|| panic!("no node `{id}`"));
    let scene = h.ui.scene.borrow();
    std::iter::once(node).chain(scene.ancestors(node)).all(|n| {
        scene.style(n).layout_options.as_deref().and_then(|o| o.display)
            != Some(zgx::layout::Display::None)
    })
}

/// How much of node `id` shows: its opacity and every ancestor's.
fn seen(h: &mut Harness, id: &str) -> f32 {
    let node = h.find(id).unwrap_or_else(|| panic!("no node `{id}`"));
    let scene = h.ui.scene.borrow();
    std::iter::once(node)
        .chain(scene.ancestors(node))
        .map(|n| scene.effects(n).opacity)
        .product()
}

#[test]
fn with_reduce_motion_a_push_and_a_pop_cross_fade_as_uikits_do() {
    let mut h = Harness::inline(
        r#"local ios = ui.ios
        return ios.stack {
            root = function(nav)
                return ui.column {
                    ui.text { "Root", id = "root" },
                    ui.button { "Go", id = "go", on_click = function()
                        nav.push(function(inner)
                            return ui.button { "Back", id = "back", on_click = function() inner.pop() end }
                        end)
                    end },
                }
            end,
        }"#,
    );
    h.ui.theme().set_system_preferences(zgx::theme::SystemPreferences {
        reduced_motion: true,
        ..Default::default()
    });
    h.frames(2);
    assert_eq!(h.error(), None);
    h.click("go");
    h.advance_ms(17.);
    // No slide: the new page is in place, fading in over the old.
    let early = seen(&mut h, "back");
    assert!(early < 0.3, "fading in from nothing: {early}");
    assert!(h.bounds("back").x.abs() < 0.5, "in place, not sliding in");
    h.advance_ms(250.);
    let (incoming, outgoing) = (seen(&mut h, "back"), seen(&mut h, "root"));
    assert!((0.25..0.85).contains(&incoming), "halfway in: {incoming}");
    assert!((0.15..0.75).contains(&outgoing), "halfway out: {outgoing}");
    h.advance_ms(400.);
    assert!((seen(&mut h, "back") - 1.).abs() < 0.01);
    assert!(!displayed(&mut h, "root"), "the page under goes once it has faded");
    // A pop: the root fades back in, the page goes when it has faded.
    h.click("back");
    h.advance_ms(267.);
    let root = seen(&mut h, "root");
    assert!((0.25..0.85).contains(&root), "fading back in: {root}");
    h.advance_ms(400.);
    assert!(h.find("back").is_none(), "the popped page is gone");
    assert!((seen(&mut h, "root") - 1.).abs() < 0.01);
}
