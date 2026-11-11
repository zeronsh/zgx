---
section: performance
---
- **A table given to state is read with one allocation of its own** (zgx-script): each list and map a state was given gathered its items in a vector of its own, then copied them into its value (a list twice); the walk now gathers every table's on two stacks it reuses, and each table's are moved off into one allocation. A 10,000-message feed given to `ui.state` converts in 18-23 ms, not 22-29 (headless, M5 Max).
