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
