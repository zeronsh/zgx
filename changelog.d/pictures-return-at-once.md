---
section: fixed
---
- **A picture scrolled back into a list draws at once** (zgx-gpu): its row's removal took its texture, and its decoded pixels were long given up, so a large picture showed an empty box until it decoded again, every time it returned. Its texture now stays, up to 32 MiB of them, until the window goes idle.
