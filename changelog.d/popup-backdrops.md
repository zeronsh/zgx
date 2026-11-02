---
section: fixed
---
- **A web view's diagnostics report a popup window's backdrop** (zgx-desktop, macOS): `NativeDiagnostics::backdrops` lists AppKit's material behind each translucent popup shown in a window of its own (a menu past its window's edge) as an `NSVisualEffectView` at the popup's box, so `web_view --fixture`'s menu backdrop check passes again; it now runs on every macOS CI job.
