---
section: fixed
---
- **A DOM view's tasks and timers run in the browser** (zgx-dom): `cx.tasks()` no longer panics there, and a Luau app's `ui.after`, `ui.every`, `ui.persist` saves and promises (`ui.call`, `ui.fetch`, file reads) run, where they never started before. The page's event loop runs them: one `setTimeout` waits for the earliest timer, and an idle page has none armed.
