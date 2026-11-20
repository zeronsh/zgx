---
section: added
---
- **A view with a context menu answers AXShowMenu** (zgx, zgx-desktop): VoiceOver's Show Menu (VO-Shift-M), and the equivalents on other platforms, opened no context menu, since only a pointer or Shift+F10 did. A view with `context_menu` now offers `ShowContextMenu`, which focuses it and sends it the Menu key, the path Shift+F10 takes.
