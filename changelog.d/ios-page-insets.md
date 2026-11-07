---
section: fixed
---
- **An iOS page ends where UIKit's does, and its scroll indicator keeps to UIKit's insets** (zgx-script): a page's last row stops the safe area clear of the bottom, or a tab bar's whole height with a `ui.native_tabs` mounted (`ui.window.tab_bar`, the system's height), where it left 90 points more than any of them. `indicator_insets` on a scroll view says how far its indicator stays inside its edges; `ios.page` sets it clear of the small bar and of what covers the bottom.
