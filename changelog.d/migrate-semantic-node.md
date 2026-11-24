---
section: changed
---
- **A `SemanticNode` is made with `SemanticNode::new` and then assigned to** (zgx). It has a private field now, so a literal ending in `..SemanticNode::new(role, label)` no longer compiles, whichever fields it names: `let mut node = SemanticNode::new(role, label); node.disabled = true;`.
  Fourteen fields few nodes set (`text_selection`, `logical_parent`, `min`, `max`, `numeric_value`, `position_in_set`, `size_of_set`, `placeholder`, `document_selection`, `level`, `description`, `shortcut`, `active_descendant`, `table`) are in a `SemanticRare` it derefs to. Reading one, assigning one, `take()`, `clone` and `==` compile as they did. These don't:
  - moving one out, `let d = node.description`: `node.description.clone()`, or `node.description.take()` on a `mut` node;
  - a pattern naming one, `let SemanticNode { level, .. } = &node`: `let level = &node.level`;
  - two borrowed mutably at once, `(&mut node.min, &mut node.max)`: assign one, then the other.
- **`SemanticNode` and `Semantics` stay on their thread** (zgx): neither is `Send` or `Sync` now, as nodes share their rarer fields. To hand accessibility information to another thread, copy out what it needs (the role, the label's `String`, the numbers) and send that.
