---
section: fixed
---
- **An effect added while another is due to come off later is refused, not lost** (zgx-audio): an effect taken off at a later frame now holds its place on its bus until then, so `add_effect` on a full bus returns `TooManyEffects` instead of handing back a handle to an effect the mixer had to throw away. An effect taken off before a change of rate reached it no longer comes back to run for ever.
