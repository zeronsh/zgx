---
section: changed
---
- **A chosen menu item no longer flashes on Windows** (zgx): where menus are windows of their own, a chosen item flashed off and on (100 ms) before its menu closed and its action ran, as AppKit's menus do; on Windows the menu now closes and the action runs at once, as WinUI's. macOS and Linux keep the flash.
