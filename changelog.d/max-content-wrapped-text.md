---
section: fixed
---
- **Wrapped text fills the room it wrapped in** (zgx): a box that shrinks to its content (the default, `fit-content`, `max-content`) under a `max_w` took the width of its widest wrapped line, so a paragraph that only just wrapped left the box short of its cap. A wrapped text now measures as wide as the room it wrapped in when its unwrapped width exceeds it, as in CSS; text that fits keeps its own width.
