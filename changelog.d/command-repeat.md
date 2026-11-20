---
section: changed
---
- **A held shortcut runs its command once** (zgx-desktop, zgx-script): the key's auto-repeat is taken and dropped, in zgx's keymap (`KeyBinding::once`) and in AppKit's menu key equivalents, as web one-shot hotkeys ignore `KeyboardEvent.repeat`. `ui.command { repeat = true }` keeps the repeat (history back and forward).
