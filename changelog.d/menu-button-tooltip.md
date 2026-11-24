---
section: fixed
---
- **A menu button with a tooltip opens as it is pressed** (zgx): a bare button opening a menu is a menu button, a press, Enter, Space or Down opening it; with a tooltip on it (`.tooltip`, Luau's `tooltip = …`) the menu saw the tip's wrapper, not a button, and nothing opened it. A tooltip's wrapper around a bare button now counts as one.
