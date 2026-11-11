---
section: performance
---
- **`ui.json.decode` makes its tables in a third of the time** (zgx-script): a tree of lists and maps a script is given (a decoded JSON document, a state read whole) is made into Luau tables in one call through Luau's C API, where a map holding a list (a feed's message) set each field through mlua, a closure made and a protected call each. A 10,000-message feed's tables took 65-76 ms and take 24-32 (headless, M5 Max), and its first frame comes 50 ms sooner. The same tables, counted the same.
