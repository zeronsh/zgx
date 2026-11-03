---
section: fixed
---
- **A modal opening closes the popups open outside it** (zgx, all platforms): a dropdown, menu or submenu left open stayed drawn over a `modal` (`ui.modal`) opened after it, and, where it overflowed the window, in a popup window above everything in the window (a ⌘K palette under an open dropdown). A modal opening now closes every anchored popup open outside it, in the window or in a window of its own, as AppKit closes menus and popovers when a sheet opens. Popups opened from inside the modal stay, and a `layer` (an overlay, not a dialog) closes nothing.
