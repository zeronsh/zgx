---
section: added
---
- **`ui.window.full_screen` and `ui.window.full_screen_transition`** (zgx-script, zgx-desktop): whether the app's window is in full screen, however it went in or out (the green button, Ctrl+Cmd+F, the menu, Mission Control, a script), and, while macOS animates it, which way it goes and about how long it takes (`{ phase = "entering" | "leaving", duration }`, nil otherwise and always on Linux and Windows). `ui.window.controls` is nil in full screen, so a title bar laid out around the traffic lights can slide over as they go. Rust hosts have `WindowContext::full_screen_transition` (`FullScreenTransition`) beside `WindowInfo::fullscreen`, and pass both to scripts with `ScriptHost::window_full_screen`.
