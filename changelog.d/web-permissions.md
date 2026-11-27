---
section: added
---
- **What a page asks to use is the app's to allow** (zgx, zgx-desktop): `WebView::set_permission_handler` hears each request a page makes for a camera, a microphone, the screen, the location, notifications, the clipboard and the rest an engine asks about (`PermissionKind`), with the page's URL and origin, and allows or denies it. Nothing is allowed for the app: with no handler, or a responder dropped unanswered, it's denied, and no engine keeps the decision in its profile. WebView2 and WebKitGTK ask about each; WKWebView about a camera and a microphone (macOS 12), denying the location and notifications without asking; Ladybird pages ask nothing yet. On Linux pages now have `getUserMedia`: zgx turns WebKitGTK's media streams on, behind this handler.
