---
section: performance
---
- **Luau no longer reads the clock per call in the browser** (zgx-script): with no watchdog thread there, every safepoint checked a call's time budget, reading `performance.now`. Now one check in 1,024 looks, and a call's budget starts at that first look, so a short call reads no clock at all. Once a call is out of time, every check looks again, so a runaway script is still stopped just past its budget. A Luau table's create of 10,000 rows takes 2.5% less script time. The desktop is unchanged.
