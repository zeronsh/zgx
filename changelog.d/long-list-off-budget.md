---
section: fixed
---
- **A long list reaches a script without spending its budget** (zgx-script): making a state's long list into a table (`:get()`, `:peek()`, a `ui.list`'s keys) is the host's work, and no longer counts against the 50 ms a call has. A chat's long transcript took more than that, so the next line of whatever read it stopped with `script exceeded its time budget`, a computed kept its old value and a list lost its keys.
