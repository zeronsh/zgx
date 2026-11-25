---
section: changed
---
- **`ui.press` takes no pointer lock** (zgx, zgx-script): a script's press neither grants activation nor uses a person's, so a control it presses can't lock the pointer (`el:lock_pointer()`) even just after a person pressed or typed, as a browser trusts no synthetic click. `zgx inspect click` is as it was.
