---
section: fixed
---
- **A window covering its display on Wayland is the display's size** (zgx-desktop): `WindowOptions::covers_display` sizes it in the compositor's logical pixels, as xdg-output says them, from its first frame. It asked for the display's mode as logical pixels where the compositor says a window's scale only after its first frame, as Hyprland does: over a 3840 by 2160 output scaled 1.5 it opened 3840 by 2160 of them, half as large again as the output, its centre off the output's; under sway it opened that large and shrank a frame later.
