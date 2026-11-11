---
section: added
---
- **On iOS, sliders, segmented controls and steppers are UIKit's own** (iOS). `ui.slider`, `ui.segmented` and `ui.stepper` host `UISlider`, `UISegmentedControl` and `UIStepper` with the same Luau API, unless styled beyond what UIKit takes or `native = false`. A pan starting on one scrolls as in a `UIScrollView`, and a finger held and lifted clicks as a `UIControl` fires.
