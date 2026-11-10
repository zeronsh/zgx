# zgx patch to taffy 0.9.0

Vendored from crates.io `taffy` 0.9.0 (MIT, `LICENSE.md` from the
repository's v0.9.0 tag, which the published crate leaves out) and
published as `zgx-taffy`, its library still `taffy`, so zgx's crates build
from crates.io without a `[patch]` section. The changes are marked `zgx:` in
the source.

## Why

A flex line with an auto margin (`margin_left = auto` to push items to the
end, a toolbar's spacer) lost its gaps: where free space goes to auto
margins, upstream set each item's main-axis offset to the margin alone,
dropping the `gap` between items that every other path keeps. CSS keeps
the gaps.

A child kept a size from its parent's old layout mode: a grid's child given
an aspect ratio, its parent then made flex, kept the height the grid had
sized it to (122 where flex gives it its own 36). A node's cache is keyed
by known dimensions, available space and run mode, not by which algorithm
its parent sizes it with, and changing a node's display left its children's
caches as they were.

## What

1. In `src/compute/flexbox.rs`, distributing free space to auto margins
   sets each item's offset to the gap before it (none before the first;
   the last in a reversed line), then adds its auto margins as before.
2. In `src/tree/taffy_tree.rs`, `TaffyTree::set_style` clears the caches
   of a node's children when its `display` changes (rare: a few nodes'
   caches, which the change dirties anyway).
3. In `src/compute/leaf.rs`, laying a leaf out (`RunMode::PerformLayout`)
   gives its measure function the sizes it knows, as sizing it does. With
   none, a box whose insets fix its width was laid out as one that has none
   (everything it holds measured again, its percentages of nothing) before
   zgx placed it at that width.
4. In `src/compute/flexbox.rs`, an item's minimum main size is measured
   at min-content only when it is automatic (`unwrap_or_else` where 0.9.0
   had `unwrap_or`). An item naming its minimum (`min_w = 0` beside a
   basis of 0, a chat row's text column) was laid out at zero width on
   every pass and the result dropped: a paragraph a line a word, in Chinese
   or Japanese a line a character, 83 ms for one on an iOS simulator.
   Later taffy releases take a closure there too.
5. In `src/tree/taffy_tree.rs`, a node's context is kept in slotmap's
   dense `SecondaryMap` with the `std` feature too, where upstream takes
   the sparse one, a `HashMap` keyed by slot under SipHash. A leaf looks
   its context up each time it is laid out, and in zgx's trees nearly
   every node is a leaf with one: the hashing was 0.6% of a dragged split's
   instructions.

Drop the fork when a taffy release keeps the gaps and keys children's
results by their parent's layout mode and passes known sizes to a leaf's
layout and measures an automatic minimum only (the zgx tests are
`crates/zgx/tests/layout_gaps.rs`, `layout_history.rs`,
`viewports_within.rs` and `scene.rs`'s
`a_flex_item_naming_its_minimum_is_not_laid_out_at_min_content`).
