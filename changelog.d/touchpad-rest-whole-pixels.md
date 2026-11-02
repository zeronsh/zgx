---
section: fixed
---
- **A touchpad scroll comes to rest on a whole device pixel on Linux** (zgx-desktop): a two-finger scroll and the coast after the fingers lift could leave what they moved resting between pixels (a canvas view at x 86.502), its text and hairlines blurred, where the wheel's scroll eases in whole pixels. Its last step now brings the scroll's whole motion to a whole number of device pixels, at the window's scale and zoom, by under half of one: no visible jump, and fractions still while it moves.
