---
section: added
---
- **Swipe actions on iOS rows** (`ui.ios.swipe_row`, `ui.ios.row { swipe = … }`): UIKit's iOS 26 row swipes, read from UIKitCore: 25 points across before 10 down, its button layout and reveal, stretch, full swipe and springs; a destructive action collapses the row, then `on_delete` takes the item from the app's list; a touch elsewhere closes an open row and does nothing else; VoiceOver offers the actions as the row's. `ui.native_pan` gains `across`, and `ui.custom_actions` and `ui.touch_watch` are new.
