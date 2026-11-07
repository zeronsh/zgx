---
section: added
---
- **Notifications and the badge on iOS** (iOS): `ui.notify` and `ui.badge` go through `UNUserNotificationCenter` as on macOS. Permission is asked only when an app first posts and nobody has decided (`ZGX_NOTIFY_NO_ASK=1` refuses instead, for automated runs). The center's delegate is set at launch, so a tap that launched the app reaches `on_click`. A badge is a count.
