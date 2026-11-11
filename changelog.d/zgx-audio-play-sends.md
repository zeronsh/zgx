---
section: fixed
---
- **A play's sends keep to two, as a voice's do** (zgx-audio). `Play::send` to a third bus is now ignored, as `Voice::set_send` ignores it; before, it took the second's place. Another send to a bus already sent to sets that send's level.
