---
section: performance
---
- **`ui.markdown` takes the state holding its text** (zgx-script): `ui.markdown(reply, { key = "reply" })`, in a computed, follows `reply` as `reply:get()` would, without making the text a script string. A keyed stream then parses a long reply (`reply:append(token)`) from what was appended alone, and a short one is compared without leaving Rust: each chunk of a streaming reply costs what it adds. `md-stream`, given the state, does 5.1% fewer instructions. A string works as before.
