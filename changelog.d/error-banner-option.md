---
section: added
---
- **`WindowOptions::error_banner`** (zgx-desktop): `false` keeps zgx's own "Something went wrong" banner out of the window for an app that tells failures its own way. Contained failures are still logged and still reach `zgx::issues` listeners (`ScriptHost::on_issue`). Defaults to `true`.
