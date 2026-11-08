---
section: changed
---
- **Keyboard focus on Windows is drawn as WinUI's focus visual** (zgx): a ring 2 wide, 1 outside the control, in the text's colour, over a contrasting ring 1 wide in the gap (`focus.ring.inner`, the background at 70%), its corners the control's grown by 3, bordered control or not; it takes no room. macOS and Linux keep the accent ring inside the control. An outline can have such a second ring anywhere: `Styled::outline_inner`, `Scene::set_outline_inner`. A tooltip's padding below its text is its own token, `tooltip.padding.bottom` (its padding down by default; 8 on Windows, as WinUI's 9,6,9,8).
