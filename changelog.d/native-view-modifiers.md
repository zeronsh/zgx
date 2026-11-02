---
section: fixed
---
- **`Ui::modifiers` follows the modifier keys while a web page or terminal has focus** (zgx-desktop, macOS): AppKit sends `flagsChanged:` to the key window's first responder, so while a page or a terminal held the keyboard winit heard no modifier change, and `ui.modifiers` stayed as it was (Command held for shortcut hints never showed, or a key stayed held after it was released). A local event monitor, installed with the native views, now passes each change zgx's view doesn't hear to `Ui::modifiers`.
