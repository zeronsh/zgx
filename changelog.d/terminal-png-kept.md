---
section: performance
---
- **A terminal's PNG pictures hold their file, not their pixels** (zgx-terminal): in the scrollback a picture costs its file's bytes, and is decoded only when drawn. A PNG with a good header and bad pixels now draws nothing instead of being refused, unless a program asks first with `a=q`.
