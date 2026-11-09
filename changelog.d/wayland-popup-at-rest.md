---
section: fixed
---
- **A click on a Wayland popup open more than a moment is heard at once** (zgx-desktop): with nothing left to do, the event loop slept through a popup's pointer input, a compositor's dismissal and its pings until something else woke it, so a menu open longer than 1.5 s took no click.
