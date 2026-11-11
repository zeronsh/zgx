---
section: added
---
- **The terminal reports its settings to DECRQSS** (zgx-terminal): a program asking for a setting (`DCS $ q`) is told the text's attributes (with underline styles as `4:3`), the cursor's style and the scrolling region as it would set them, and that any other isn't kept. nvim asks before drawing curly underlines, and draws them only once the answer has `4:3`; where `COLORTERM` isn't set, it turns on direct colour once the colour it set comes back.
