---
section: added
---
- **A text area draws shapes beside its lines** (zgx, zgx-script): `EditorHandle::set_line_marks` and a textarea's `line_marks = function(line, state) … end` give each line boxes (`LineMark`): a quote's rule, the wash behind a code block's line, a horizontal rule, a list item's number as a label (monospaced or in tabular figures). They're placed where their lines are, under any room above, scroll with them under the text (or over it), and are asked once for each line text and state. A text area without marks draws as before.
