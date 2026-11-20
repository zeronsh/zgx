---
section: fixed
---
- **A named state set back to nil goes once nothing holds it** (zgx-script): the sweep dropped only unheld states still at their first value, so a key the host published into and cleared (`chat:<id>`, `chat_live:<id>`) stayed for the life of the window. A nil one nothing holds goes too. `app:gc_holders(kind, options)` in `zgx test` lists what holds the objects of a kind, as `gc --retained` does.
