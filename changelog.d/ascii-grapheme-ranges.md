---
section: performance
---
- **A screen reader's text costs a third less to send** (zgx, zgx-gpu, zgx-desktop): placing carets on a text's lines and building a screen reader's text runs walked every grapheme through Unicode's segmentation, three times a character for carets. An ASCII text's graphemes now come from its bytes (`zgx::text_layout::grapheme_ranges`), so a full accessibility update of a transcript costs 32.6% less, with the same runs and carets.
