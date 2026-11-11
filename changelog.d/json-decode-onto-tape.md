---
section: performance
---
- **`ui.json.decode` reads a document straight into tables** (zgx-script): it was parsed into zgx's values, those made into tables, then dropped; it is now read onto a flat tape (each value a token in order, its strings' bytes in one buffer) and the tables made from that in one call. A 10,000-message feed decodes in 34-36 ms, not 47-54 (headless, M5 Max), and the same tables come out. An object naming a key twice keeps its last value, as before, and counts the key twice in `values_converted`.
