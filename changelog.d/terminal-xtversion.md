---
section: added
---
- **The terminal answers XTVERSION** (zgx-terminal): a program asking for the terminal's name and version (`CSI > q`, as editors and prompts do where `TERM_PROGRAM` doesn't reach, over ssh or in tmux) is told `zgx(<version>)`, or the `TERM_PROGRAM` and `TERM_PROGRAM_VERSION` an app gives its session in `SessionOptions::env`.
