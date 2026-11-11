---
section: added
---
- **The terminal answers XTGETTCAP** (zgx-terminal): a program asking for terminfo capabilities by name (`DCS + q`) is told which this terminal has: direct colour (`RGB`, `Tc`, `setrgbf`, `setrgbb`), the clipboard through OSC 52 (`Ms`), the cursor's shape, styled and coloured underlines, the name and colours; one it hasn't is answered as not found. nvim asks for the direct colour ones where `COLORTERM` isn't set (over ssh, where it rarely arrives) and turns on its own colours once they're found.
