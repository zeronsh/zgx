---
section: changed
---
- **`WindowOptions` has a new field, `max_size`** (zgx-desktop): a `WindowOptions` written out field by field stops compiling. *Migrate:* end it with `..Default::default()`, or set `max_size: None`.
