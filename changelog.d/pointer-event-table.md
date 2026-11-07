---
section: performance
---
- **A pointer event's table is made in one call** (zgx-script): each handler a pointer event reaches gets a table of `x`, `y`, `local_x`, `local_y` and the keys held. It was filled field by field from Rust, and with a memory limit set (as every script host has) each `set` was a protected call of its own, about 18 a table. A small Luau function, compiled once per state, now makes it: the same table, new for each event, with the same fields. On a chat page at 120 moves a second, a move's dispatch went from about 515k to 217k instructions.
