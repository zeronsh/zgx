---
section: added
---
- **`ui.tooltip_look`**: every text tip (`tooltip = "…"`) takes the app's panel and default placement, and `keep_below` keeps tips out of a band at the window's top (a titlebar): one that would open above its node there flips below it.
- **`ui.modal { scrim_blur }`**: what lies behind a modal's backdrop blurs (CSS `backdrop-filter` on a dialog's backdrop), its panel sharp. Opening a modal no longer resets its overlay's other effects.
