---
section: added
---
- **`dir = "auto"`: text runs the way its first strong character does** (zgx, zgx-script): Arabic, Persian and Hebrew text starts at the right (its alignment, and a container's mirrored rows, follow `Scene::direction`), English stays left to right, and text with no strong character takes its parent's; decided again before layout as the text changes. `Styled::dir_auto`, `Scene::set_direction_auto`, `zgx::text_layout::first_strong_direction`, and `ui.direction_of(text)` for a container showing that text (`dir = ui.direction_of(text)`). Bidi reordering within a line was already the shaper's. Not yet: a forced `dir = "rtl"` on text that starts with Latin letters still shapes with a left-to-right base level (its punctuation lands as in a left-to-right paragraph).
