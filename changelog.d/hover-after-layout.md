---
section: fixed
---
- **A hover card opening under the pointer no longer tells its view the pointer left** (zgx): what is under a resting pointer is found once layout has settled, not in each pass, where a popup not yet placed could lie under it. `on_hover` heard a leave and an enter as the card opened.
