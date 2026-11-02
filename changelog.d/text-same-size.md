---
section: performance
---
- **A text that changes to one measuring the same is repainted, not laid out again** (zgx): a counter or a clock (a text that doesn't wrap, as long as before) is measured as it changes, and if it measures as before nothing around it is laid out again. arm64: micro-state −20.1M instructions, script-update −16.6M, micro-state-luau −8.3M, app-dashboard −2.2M; i18n_bench's frame 87 → 34 µs (#1288).
