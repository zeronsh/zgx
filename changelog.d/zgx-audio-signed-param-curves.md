---
section: fixed
---
- **A parameter that goes below zero keeps its sign under any ramp** (zgx-audio): an equal-power or exponential curve treats values as gains, none below zero, so ramping a decibel parameter with one turned -12 → -6 dB into +12 → +6 (a ducker set deeper made its bus louder). Such a parameter now moves in straight steps whatever the curve, and every frame's value is kept to the parameter's range.
