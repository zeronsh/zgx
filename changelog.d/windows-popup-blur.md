---
section: changed
---
- **A translucent popup on Windows 11 blurs what is behind it** (zgx-desktop, zgx-gpu): menus, popovers and tooltips get the acrylic accent plain glass already has, and DWM rounds and shadows their window. `WindowCapabilities::popup_blur_behind` says whether popups blur, so an app can draw them solid where they can't.
