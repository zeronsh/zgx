---
section: performance
---
- **A layout with no column streamed into makes no call for one** (zgx): a layout pass looks at whether columns resumed at their end set children aside, or are streamed into at all, before calling what returns or restores them, so a scene with neither pays a test, not two calls a pass.
