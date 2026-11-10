---
section: fixed
---
- **`capture_keys = true` works on a node with no input handlers** (zgx-script): it did nothing without one, such as an `on_key`, so bindings and the menu bar still took the keys while focus was in it.
