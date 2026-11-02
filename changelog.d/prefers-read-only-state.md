---
section: fixed
---
- **`ui.prefers`'s states type-check when read** (zgx-script): they were typed `Readable<boolean>`, whose `get` takes `self: never` so that any state fits a bound property, which made `ui.prefers.reduced_motion:get()` an error in a strict script. They are now `ReadOnlyState<boolean>` (`get` and `peek`), as `ui.modifiers` is.
