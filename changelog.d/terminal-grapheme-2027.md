---
section: added
---
- **The engine-drawn terminal clusters graphemes as Ghostty does, DEC mode 2027** (zgx-terminal; `zgx-alacritty-terminal` 0.26.3): an emoji ZWJ sequence (👨‍👩‍👧), a flag or a skin-toned emoji is one character in two cells, where it took six, four or more; VS15 makes an emoji one cell wide. The mode is on until a program resets it (`CSI ? 2027 l`), and DECRQM answers it: reset, each character takes its own width as `wcwidth` counts, so a shell's line editor keeps its cursor in step across ❤️.
