---
section: changed
---
- **An emoji asked for with VS16 takes two cells in the engine-drawn terminal** (zgx-terminal; `zgx-alacritty-terminal` 0.26.2): ❤️, ✔️ or ⚠️ (a character that is text by default, then U+FE0F) took one cell and now takes two, as Ghostty, kitty and iTerm2 show it and as programs measuring with Unicode's widths count it (Ink's string-width, ratatui), so coding agents' lines stay in step. A shell's line editor counting with `wcwidth` sees one column, as in those terminals: an edit across such an emoji puts its cursor a cell off.
