---
section: added
---
- **An app's own Window and Help menus are AppKit's, and full screen is a standard item** (zgx-desktop, macOS): a menu titled "Window" keeps Minimize and Zoom above the app's items and Bring All to Front below them, as AppKit apps' do (they were left out as soon as an app had a Window menu of its own), except one the app put there itself; a menu titled "Help" becomes AppKit's help menu, which holds its menu search. `StandardItem::ToggleFullScreen` is AppKit's Enter Full Screen item (⌃⌘F, titled Exit Full Screen while the window is); elsewhere it toggles the focused window's full screen.
