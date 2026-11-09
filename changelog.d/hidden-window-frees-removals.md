---
section: fixed
---
- **A window that isn't presenting frees what it removed** (zgx, zgx-desktop): a large removal's nodes were freed over later frames, so in a hidden, minimized or covered window they stayed, and the idle trim never ran, until it next drew. The host now frees them with no frame, then trims. `zgx::idle::request` asks for the idle trim from anywhere, with no frame.
