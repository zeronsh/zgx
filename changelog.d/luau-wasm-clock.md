---
section: performance
---
- **Luau reads the clock far less in the browser** (zgx-script): with no watchdog thread there, every safepoint checked a call's time budget, reading `performance.now` twice. One check in 1,024 looks now, and every check once a call is out of time, so a runaway script is still stopped just past its budget. That's about a third fewer clock reads across a Luau table's 10,000 rows. The desktop is unchanged.
