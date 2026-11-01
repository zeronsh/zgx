---
section: fixed
---
- **A drag begun as the previous drop finishes isn't lost on Wayland** (zgx-desktop): the compositor gives the pointer back at a drop, before the drop's target finishes with it. A drag started then (a tab, say) was ended by that earlier drop's end, with a button release it never had. It now goes on, and goes to the compositor once the earlier drop is done. *Known issue:* until then it can't leave its window, so let go within that moment (a few milliseconds, longer for a slow target), it drops nothing ([#1259](https://github.com/zeronsh/zgx/issues/1259)).
