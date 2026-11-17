---
section: added
---
- **A text area's code blocks scroll sideways and never wrap** (zgx): a line marked `LineInset::nowrap` in a scroll group shows at its full width and scrolls with the group, as the web's `white-space: pre` with `overflow-x: auto`; `LineMark::pinned` keeps a block's surface and line numbers still as its text scrolls; `LineMark::over_text` draws a mark above the text (a block's actions); `View::group_scrollbar` shows the scroll views' scrollbar across the bottom of the block that moved last, its drag scrolling it (all hidden while they settle). Text outside a block wraps as before and pays nothing.
