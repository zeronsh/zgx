---
section: added
---
- **Refused sends are counted** (zgx-audio). A send to a third bus while a voice sends to two is refused, whether it comes from `Play::send` or `Voice::set_send`, and counted in `Stats::sends_refused`.
