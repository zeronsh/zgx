---
section: fixed
---
- **A terminal answers a program's queries in the order it asked** (zgx-terminal): a colour query (OSC 4, 10, 11, 12) was answered at the end of each read, after the cursor and device reports asked after it. termenv (gh, glamour, lipgloss) asks for the background, OSC 11, then the cursor, CSI 6n, took the cursor report first as "no colour" and stopped reading, and the colour's answer that followed read as typed keys (gh's prompt: "unexpected escape sequence from terminal"). Every answer now goes back in the order asked, in one write, each colour with its query's terminator. Linux and Windows; macOS's Ghostty terminals already did.
