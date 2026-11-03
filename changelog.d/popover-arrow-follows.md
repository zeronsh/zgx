---
section: fixed
---
- **A popover's arrow is painted as its panel is, as it changes** (zgx): the arrow took a copy of the panel's styles as it mounted, so a panel whose background, border or opacity followed state (a reactive style) drew its arrow in the colour it had first. The arrow takes what the panel's reactive style gives each time it runs.
