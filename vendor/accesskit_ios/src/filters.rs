// Copyright 2026 The AccessKit Authors. All rights reserved.
// Licensed under the Apache License, Version 2.0 (found in
// the LICENSE-APACHE file) or the MIT license (found in
// the LICENSE-MIT file), at your option.

use accesskit_consumer::{FilterResult, Node};

pub(crate) use accesskit_consumer::common_filter as filter;

use crate::node::NodeWrapper;

/// Filter for determining if a node should be an accessibility element.
/// On iOS, a node with focusable children must NOT be an accessibility element
/// (must return false for isAccessibilityElement), otherwise VoiceOver will
/// ignore its children entirely.
///
/// A node that has its own interaction semantics — actions, a value, or a
/// toggled state — is always a leaf accessibility element. Its descendants
/// (e.g. a `Role::Label` child) are collapsed into the parent's
/// `accessibilityLabel` via `accesskit_consumer::Node::labelled_by`, so
/// exposing them separately would let VoiceOver focus the label and skip
/// the actionable parent.
///
/// Otherwise, non-focusable children (e.g. Labels, Images) just provide
/// labeling info to the parent, so the parent should remain the
/// accessibility element.
pub(crate) fn filter_for_is_accessibility_element(node: &Node) -> FilterResult {
    let result = filter(node);
    if result != FilterResult::Include {
        return result;
    }

    let wrapper = NodeWrapper(node);
    // zgx patch: taking focus alone (a dialog, a menu, a grid, a list item, for
    // a keyboard) makes no leaf of what holds controls or groups of its own: a
    // sheet's grabber and content were hidden inside one empty element.
    if !wrapper.has_own_action()
        && node.toggled().is_none()
        && !node.has_value()
        && node.numeric_value().is_none()
        && node.filtered_children(&filter).any(|child| {
            NodeWrapper(&child).has_non_scroll_action() || child.filtered_children(&filter).next().is_some()
        })
    {
        return FilterResult::ExcludeNode;
    }
    if wrapper.has_non_scroll_action()
        || node.toggled().is_some()
        || node.has_value()
        || node.numeric_value().is_some()
    {
        return FilterResult::Include;
    }

    // zgx patch: a heading holding nothing but its text is one element, as a
    // web page's `h1` and UIKit's title label are: its text names it and its
    // header trait goes to VoiceOver's headings rotor. As a container, its
    // text alone was focused, read as plain static text.
    if node.role() == accesskit::Role::Heading
        && node.filtered_children(&filter).all(|child| {
            child.role() == accesskit::Role::Label && child.filtered_children(&filter).next().is_none()
        })
    {
        return FilterResult::Include;
    }

    // If this node has any filtered children that are focusable or are
    // themselves containers (have their own filtered children), it should be
    // a container, not an accessibility element.
    if node.filtered_children(&filter).any(|child| {
        NodeWrapper(&child).can_be_focused() || child.filtered_children(&filter).next().is_some()
    }) {
        return FilterResult::ExcludeNode;
    }

    FilterResult::Include
}

/// zgx patch: the modal node shown last in tree order (a sheet, a dialog) under
/// `node`, if one is: what VoiceOver alone reads while it shows, as UIKit reads
/// only a view that is `accessibilityViewIsModal` and what it holds.
pub(crate) fn modal_root<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut found = None;
    for child in node.filtered_children(&filter) {
        if child.is_modal() {
            found = Some(child);
        } else if let Some(inner) = modal_root(&child) {
            found = Some(inner);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use accesskit::{Action, Node as NodeBuilder, NodeId, Role, Tree, TreeId, TreeUpdate};

    const ROOT_ID: NodeId = NodeId(0);
    const CHILD_1_ID: NodeId = NodeId(1);

    fn build_tree(nodes: Vec<(NodeId, NodeBuilder)>) -> accesskit_consumer::Tree {
        let update = TreeUpdate {
            nodes,
            tree: Some(Tree::new(ROOT_ID)),
            tree_id: TreeId::ROOT,
            focus: ROOT_ID,
        };
        accesskit_consumer::Tree::new(update, false)
    }

    fn filter_node(nodes: Vec<(NodeId, NodeBuilder)>, target: NodeId) -> FilterResult {
        let tree = build_tree(nodes);
        let node = tree
            .state()
            .node_by_tree_local_id(target, TreeId::ROOT)
            .unwrap();
        filter_for_is_accessibility_element(&node)
    }

    fn make_button(label: &str) -> NodeBuilder {
        let mut node = NodeBuilder::new(Role::Button);
        node.set_label(label);
        node.add_action(Action::Click);
        node
    }

    #[test]
    fn leaf_button_is_element() {
        let mut root = NodeBuilder::new(Role::Window);
        root.set_children(vec![CHILD_1_ID]);
        let child = make_button("OK");
        assert_eq!(
            filter_node(vec![(ROOT_ID, root), (CHILD_1_ID, child)], CHILD_1_ID),
            FilterResult::Include,
        );
    }

    #[test]
    fn hidden_node_excluded() {
        let mut root = NodeBuilder::new(Role::Window);
        root.set_children(vec![CHILD_1_ID]);
        let mut hidden = make_button("Hidden");
        hidden.set_hidden();
        assert_ne!(
            filter_node(vec![(ROOT_ID, root), (CHILD_1_ID, hidden)], CHILD_1_ID),
            FilterResult::Include,
        );
    }

    #[test]
    fn checkbox_with_label_child_is_leaf() {
        const CHECKBOX_ID: NodeId = NodeId(1);
        const LABEL_ID: NodeId = NodeId(2);
        let mut root = NodeBuilder::new(Role::Window);
        root.set_children(vec![CHECKBOX_ID]);
        let mut checkbox = NodeBuilder::new(Role::CheckBox);
        checkbox.add_action(Action::Click);
        checkbox.set_children(vec![LABEL_ID]);
        let mut label = NodeBuilder::new(Role::Label);
        label.set_value("Accept terms");
        assert_eq!(
            filter_node(
                vec![(ROOT_ID, root), (CHECKBOX_ID, checkbox), (LABEL_ID, label),],
                CHECKBOX_ID,
            ),
            FilterResult::Include,
        );
    }

    #[test]
    fn heading_holding_its_text_is_one_element() {
        const HEADING_ID: NodeId = NodeId(1);
        const LABEL_ID: NodeId = NodeId(2);
        let mut root = NodeBuilder::new(Role::Window);
        root.set_children(vec![HEADING_ID]);
        let mut heading = NodeBuilder::new(Role::Heading);
        heading.set_label("Settings");
        heading.set_children(vec![LABEL_ID]);
        let mut label = NodeBuilder::new(Role::Label);
        label.set_value("Settings");
        assert_eq!(
            filter_node(
                vec![(ROOT_ID, root), (HEADING_ID, heading), (LABEL_ID, label)],
                HEADING_ID,
            ),
            FilterResult::Include,
        );
    }

    #[test]
    fn heading_holding_a_button_stays_a_container() {
        const HEADING_ID: NodeId = NodeId(1);
        const BUTTON_ID: NodeId = NodeId(2);
        let mut root = NodeBuilder::new(Role::Window);
        root.set_children(vec![HEADING_ID]);
        let mut heading = NodeBuilder::new(Role::Heading);
        heading.set_children(vec![BUTTON_ID]);
        assert_eq!(
            filter_node(
                vec![
                    (ROOT_ID, root),
                    (HEADING_ID, heading),
                    (BUTTON_ID, make_button("Edit")),
                ],
                HEADING_ID,
            ),
            FilterResult::ExcludeNode,
        );
    }

    #[test]
    fn dialog_holding_a_button_is_a_container() {
        const DIALOG_ID: NodeId = NodeId(1);
        const BUTTON_ID: NodeId = NodeId(2);
        let mut root = NodeBuilder::new(Role::Window);
        root.set_children(vec![DIALOG_ID]);
        let mut dialog = NodeBuilder::new(Role::Dialog);
        dialog.add_action(Action::Focus);
        dialog.set_children(vec![BUTTON_ID]);
        assert_eq!(
            filter_node(
                vec![
                    (ROOT_ID, root),
                    (DIALOG_ID, dialog),
                    (BUTTON_ID, make_button("Sheet Grabber")),
                ],
                DIALOG_ID,
            ),
            FilterResult::ExcludeNode,
        );
    }

    #[test]
    fn list_item_holding_its_text_is_one_element() {
        const ITEM_ID: NodeId = NodeId(1);
        const LABEL_ID: NodeId = NodeId(2);
        let mut root = NodeBuilder::new(Role::Window);
        root.set_children(vec![ITEM_ID]);
        let mut item = NodeBuilder::new(Role::ListItem);
        item.add_action(Action::Focus);
        item.set_children(vec![LABEL_ID]);
        let mut label = NodeBuilder::new(Role::Label);
        label.set_value("Inbox");
        assert_eq!(
            filter_node(
                vec![(ROOT_ID, root), (ITEM_ID, item), (LABEL_ID, label)],
                ITEM_ID,
            ),
            FilterResult::Include,
        );
    }

    #[test]
    fn a_modal_dialog_is_all_that_is_read_while_it_shows() {
        const PAGE_ID: NodeId = NodeId(1);
        const DIALOG_ID: NodeId = NodeId(2);
        const BUTTON_ID: NodeId = NodeId(3);
        let mut root = NodeBuilder::new(Role::Window);
        root.set_children(vec![PAGE_ID, DIALOG_ID]);
        let mut dialog = NodeBuilder::new(Role::Dialog);
        dialog.set_modal();
        dialog.set_children(vec![BUTTON_ID]);
        let tree = build_tree(vec![
            (ROOT_ID, root),
            (PAGE_ID, make_button("Wi-Fi")),
            (DIALOG_ID, dialog),
            (BUTTON_ID, make_button("Sheet Grabber")),
        ]);
        let state = tree.state();
        let found = modal_root(&state.root()).map(|node| node.id());
        let dialog_id = state
            .node_by_tree_local_id(DIALOG_ID, TreeId::ROOT)
            .unwrap()
            .id();
        assert_eq!(found, Some(dialog_id));
    }

    #[test]
    fn with_no_modal_node_everything_is_read() {
        let mut root = NodeBuilder::new(Role::Window);
        root.set_children(vec![CHILD_1_ID]);
        let tree = build_tree(vec![(ROOT_ID, root), (CHILD_1_ID, make_button("Wi-Fi"))]);
        assert!(modal_root(&tree.state().root()).is_none());
    }
}
