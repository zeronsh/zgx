---
section: fixed
---
- **A runaway Luau script in the browser is stopped about a millisecond after its time budget, however slow each step is** (zgx-script, zgx-dom): the budget check read the clock once in 1,024 safepoints, so a loop of slow steps (a long `string.rep`) ran seconds past a 30 ms budget. Its stride is now paced to about 1 ms between looks, and a call that ends early still reads no clock. A state write off the script's clock reads the clock twice, not four times, and each stopped call's account (`last_budget_stop`) is its own.
