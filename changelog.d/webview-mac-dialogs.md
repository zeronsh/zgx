---
section: fixed
---
- **A web page's `alert`, `confirm` and `prompt` show on macOS** (zgx-desktop): each is an alert sheet on the page's window, with the page's origin under its message and a field for `prompt`, and the page's script waits for its answer. They returned at once before: `alert` showed nothing, `confirm` was false and `prompt` null.
