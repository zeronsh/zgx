---
section: fixed
---
- **Popup windows open at their trigger in a zoomed window** (zgx-desktop, macOS, Windows, Wayland): in a window zoomed off 100% (`Ui::set_zoom`), a menu or popover shown in a window of its own opened away from its anchor and 1/zoom too large, as its frame was built from the scene's points with no zoom. On macOS and Wayland its window's frame, size, material and arrow, its renderer's scale and the input coming back from it now go through the window's zoom; on Windows the screens it may open on do too (its frame and input already did; on X11 everything already did). A zoom an app sets before its window exists now reaches that window's popups as well.
