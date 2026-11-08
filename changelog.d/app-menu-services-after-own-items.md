---
section: fixed
---
- **The Services submenu stays where it is when an app's own items lead the application menu** (zgx-desktop, macOS): AppKit's services were put on the application menu's first item, which with `APPLICATION_MENU` items is the app's own (About, Check for Updates…), so that item became the Services submenu and Services stayed empty. The menu bar's Services item is now found by its title, after the app's items.
