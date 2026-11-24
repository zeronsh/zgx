---
section: fixed
---
- **Lines leaving their scroll group while they show no longer panic** (zgx): deleting a code block's fence, or the block, in a text area whose lines were in a scroll group (`LineInset::scroll_group`) removed the group's layer with line nodes the text area still held, and its next draw panicked ("stale scene node"). Their nodes go with the layer now, an area whose insets are taken away shows its lines outside their old group's layer, and its group scrollbar goes.
