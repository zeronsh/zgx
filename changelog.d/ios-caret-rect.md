---
section: fixed
---
- **iOS's caret sits where UIKit's does** (zgx-desktop): a point left of the text's edge on a whole pixel, inside the view, as `caretRectForPosition:` answers in UITextField and UITextView. It was on the exact edge, a point to the right.
