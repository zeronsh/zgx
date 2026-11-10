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

A clean child answered for a question it was never asked. Taffy's cache
takes a size it was asked with to be one it answered (`known_dimensions`
equal to an entry's result), assuming a box laid out at the size it reported
reports that size again. CSS's min-content size holds to that (its height is
the one at its width); a zgx box's need not: a row answering 369 by 18.3 at
its least (width 0, its text unwrapped, wider than the room) is 36.6 high
told it is 369 wide, its text wrapping there. A container laid out twice (a
probe, then its layout) answered its real layout from what the probe had
stored, where one built whole asks; the row kept a line too few.

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
5. In `src/tree/cache.rs` and `src/tree/taffy_tree.rs`, `TaffyView`'s
   `cache_get` (`Cache::get_with`) doesn't take a width an entry answered
   at min-content for that width known, in both run modes, for a node
   measured by its context whose style leaves its width to its content
   (`auto`): a size it answered at its least is no answer at that size.
   Every other match is upstream's: a box taffy lays out answers at its
   least as CSS's min-content size does, and so does a leaf whose style
   gives its width (a percentage too, resolved), or along its height.
   (Refusing every such match cost layout-drag 1%: its 100%-wide leaves
   measured again for nothing.)

Drop the fork when a taffy release keeps the gaps and keys children's
results by their parent's layout mode and passes known sizes to a leaf's
layout and measures an automatic minimum only and takes no cached size
answered at its least for that size known (the zgx tests are
`crates/zgx/tests/layout_gaps.rs`, `layout_history.rs`,
`viewports_within.rs`, `layout_changed_as_built.rs` and `scene.rs`'s
`a_flex_item_naming_its_minimum_is_not_laid_out_at_min_content`).
