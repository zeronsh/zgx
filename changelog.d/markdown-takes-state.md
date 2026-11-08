---
section: performance
---
- **`ui.markdown` given a state answers a state of its blocks** (zgx-script): `local blocks = ui.markdown(reply, { key = "reply" })`, drawn by `ui.each(blocks, …)`, keeps the blocks current in Rust as `reply` grows. No script string is made of the text, no script table of each answer, and a keyed stream parses a long reply (`reply:append(token)`) from what was appended alone: each chunk of a streaming reply costs what it adds. A string works as before (a table of blocks). `zgx lint` says so when a computed wraps the state form (`computed-markdown-state`), and no longer takes `ui.markdown(state:get())` in a computed for a value read once.
