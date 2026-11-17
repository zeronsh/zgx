---
section: added
---
- **`zgx lint` reports a row's branches built whether they show or not** (zgx-script): the new `eager-branch` rule finds a `ui.show` or `ui.switch` branch built straight in a list's or `ui.each`'s row, which every row builds even when it never shows (an editor while renaming, a retry button after a failure).
