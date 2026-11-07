---
section: fixed
---
- **A tap on the caret opens the edit menu on iOS** (zgx, zgx-desktop), as UITextField's does: the touch also reached the editor's own press, which moved the caret first, so UIKit saw no tap on the caret. Select now shows its highlight and handles too.
