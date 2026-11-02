---
section: added
---
- **A shortcut recorder takes every key** (zgx, zgx-script, zgx-desktop): `capture_keys = true` on a node (`View::capture_keys`, `InputDispatcher::set_capture_keys`) sends every key to what is focused while focus is in it: no key binding takes one, and on macOS the menu bar's key equivalents (⌘Q, ⌘C, the app's commands) don't fire, the key going to zgx's view. A settings page records ⌘J as ⌘J instead of running its command. As focus leaves, shortcuts work again; a host asks with `InputDispatcher::captures_keys`, and `binds_key` answers no meanwhile.
