---
section: fixed
---
- **A reload keeps focus where it was even when its load moves it first** (zgx, zgx-script): where focus is is now taken before the scripts run, not after. A load that sets state the shown interface reads (a `ui.presence` keyed by it, its focused button ending) moved focus to the next stop before the old tree was replaced, and the reload kept it there. `switch_keeping_focus_held` and `FocusHold` give a host the same: take where focus is before running what may move it, and the switch finds it again in the branch it builds.
