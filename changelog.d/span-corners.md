---
section: fixed
---
- **Rounded span backgrounds round on their own corners** (zgx-gpu): a span's rounded background (`TextSpan::radius`, Luau `ui.span { bg, padding, radius }`, an inline code chip) rounded only the corners that lay on its text's own edges, so a chip mid-line was square, as was each piece of a wrapped one. Each piece now rounds on its own box: a chip wrapped across lines rounds each line's piece on its own ends, as CSS's `box-decoration-break: clone`.
