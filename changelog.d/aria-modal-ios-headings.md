---
section: fixed
---
- **On iOS, VoiceOver reads headings as headings and an open sheet alone** (iOS). A text with a style that changes (a function, a tween) lost the header trait and dropped out of the headings rotor. A dialog, menu, grid or list item holding controls of its own read as one empty element. With a dialog that says it's modal up, the page behind was read too. `aria_modal = true` (`View::accessible_modal`) marks a sheet or dialog an app builds itself; `ui.ios.sheet` sets it, hides its dimming from VoiceOver and closes on VoiceOver's two-finger scrub. A pushed page's back button is read first and is named for the page it goes back to, as UIKit's is.
