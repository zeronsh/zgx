---
section: added
---
- **An app's clipboard can be its own, and `zgx inspect` drives it** (zgx-desktop, zgx-script, zgx). Under `ZGX_CLIPBOARD=memory` a window's clipboard is held in memory: text, HTML, files, an image and the primary selection. Copying and pasting never touch the person's clipboard, for QA and off-screen sessions. `ZGX_OFFSCREEN` sessions hold theirs in memory unless `ZGX_CLIPBOARD` names another (`system`). `zgx inspect clipboard` reads it, and `zgx inspect clipboard set --text … | --html … | --image file.png` sets it. `zgx inspect key cmd+v` (`ctrl+v` off macOS) now pastes as a window's key does: an editor's `on_paste` hears it, which a key sent to the view alone never reached. ⌘C and ⌘X work the same way (`Ui::clipboard_shortcut`, hidden).
