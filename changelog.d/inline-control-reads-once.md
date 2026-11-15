---
section: fixed
---
- **A labelled control inline in a paragraph is read once, where it stands** (zgx, zgx-desktop): a citation's `ui.span { "1", label = "Citation 1", on_click = … }` in `ui.rich` was read twice, its "1" in the paragraph's text and a button named "Citation 1" after it. As a browser reads an inline link, it is now the button alone, between the text around it: the paragraph reads "step ." with the button between "step " and ".". It stays drawn and selectable, and a copy keeps its text. A span without a `label` reads as before.
