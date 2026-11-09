---
section: added
---
- **`leading` and `ui.ios.text { style = … }` set text as UIKit's text styles do** (iOS): points between lines after each but the last, with the first line the font's own height. `ui.ios.text` takes each style's size, weight and leading from the system, following Dynamic Type. iOS only: elsewhere `leading` does nothing and text keeps its normal line, since reading it there costs text that never sets it.
