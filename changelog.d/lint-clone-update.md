---
section: added
---
- **`zgx lint`'s `update-copy` sees a clone for one change** (zgx-script): `local copy = table.clone(state:peek())`, one change to it, then `state:set(copy)` is `state:update` written out, and costs as much: the whole value through Luau for each change. It says what does the change alone.
