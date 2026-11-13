---
section: added
---
- **A frozen clock for screenshot runs** (zgx, zgx-script, zgx-desktop): `ZGX_FREEZE_TIME=1` at launch, or `zgx inspect clock freeze`, holds every animation, tween, timer (`ui.after`, `ui.every`), caret blink, tooltip delay and frame at one time (a virtual clock on 1 January 2000), so spinners, carets and transitions are the same each run; `zgx inspect clock advance <ms>` moves it on by hand, waking the timers due by then in order, and `clock thaw` lets it run. Frames delivered to a `FrameClock` under a virtual clock carry its time, and baked loops (macOS) draw each tick while it is virtual. `zgx::clock::{freeze, thaw, advance_frozen, is_frozen}`.
