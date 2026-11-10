---
section: added
---
- **`ui.map(state, { from = { a, b }, to = { c, d }, clamp })`** (zgx-script): a number mapped from a state in Rust, for a style that follows it with no script run as the state changes. A scrollbar's thumb follows a list's offset (`top = ui.map(offset, { from = { 0, limit }, to = { 4, finish }, clamp = true })`) with no script per frame. Each end is a number, a state or a function kept current as what it reads changes; a range of no width gives `c`. It sets numbers and lengths.
