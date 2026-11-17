---
section: fixed
---
- **The browser gives back what a page let go of once it's still** (zgx-dom): a DOM view now has the desktop's idle trim (`zgx::idle`), run 1.5 to 3 s after the DOM was last written. A Luau app's cleared rows are collected there rather than held until its script next allocates a lot. A Luau table that showed 10,000 rows held 18.3 MB live after a clear; it now holds 3.7 MB. Nothing is armed once the trim has run, so a page at rest still runs nothing.
