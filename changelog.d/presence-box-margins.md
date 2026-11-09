---
section: changed
---
- **A `ui.presence` box sized by its content holds the content's margins** (zgx-script): as CSS sizes a grid track by an item's margin box, the box is as tall as its content with its margins, and content aligning itself (`self = "center"`) centres that margin box. The one-cell grid it was before left vertical margins out (taffy's `1fr` row in a box of no height), so such a box was shorter than its content and centred content sat a margin higher.
