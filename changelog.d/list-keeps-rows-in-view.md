---
section: fixed
---
- **Items added above a keyed list's view no longer push its rows down** (zgx, zgx-script): a `ui.list` with `key` and `measured` (or `heights`) keeps the rows in view where they are as items are inserted or removed above them, as a browser's scroll anchoring does, where before it kept its offset and the content moved under the reader. Resting at the top it doesn't anchor, so a feed's newest items show. Only a change of count looks for the row; a list nothing is added to does no more work.
