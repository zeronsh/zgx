---
section: added
---
- iOS, for now from a checkout (`scripts/ios/sim.py`, [docs/ios.md](docs/ios.md)): the desktop host runs there, Rust and Luau apps alike, in UIKit's scrolling, keyboard, clipboard, colours and text sizes.
- `glass` and `glass_group` (`ui.glass`, `ui.glass_group`): the system's Liquid Glass on iOS, between zgx's planes, and a blur under a tint elsewhere. `ui.symbol`, SF Symbols drawn by iOS. `native_toggle` and `native_tabs` (`ui.native_switch`, `ui.native_tabs`): iOS's own controls, each one element to assistive technology, and drawn elsewhere. `ui.window.safe_area` and `ui.window.keyboard`.
- On iOS: `ui.ios` (a list, page, navigation stack and sheet of iOS's measures), `ui.alert`, `ui.action_sheet`, `ui.haptic`, `ui.share`, `ui.system_menu` (UIKit's context menu, and VoiceOver's actions), `ui.on_escape`, `ui.webview` (WKWebView), `ui.open_files` (UIKit's pickers), `ui.on_open` (links), and UIKit's text selection in editors. Drawing planes stack between native views, and frames reach 120 Hz while drawing.
- `ui.scroll_edge`: iOS 26's scroll edge effect on a window's edge, and a drawn fade elsewhere. `ui.pull_refresh` is `UIRefreshControl`, and `ui.native_pan` a pan iOS recognizes.
