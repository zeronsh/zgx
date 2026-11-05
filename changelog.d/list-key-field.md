---
section: performance
---
- **`ui.list { data = s, key = "id" }` keys a long list without making its items tables** (zgx-script): the field is read in Rust, so only the rows being built reach the script. With a function, every new item is converted to read its key: at 2,000 turns of a chat, 84–88 ms a list on an M5.
