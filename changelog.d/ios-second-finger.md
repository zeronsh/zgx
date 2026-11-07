---
section: fixed
---
- **On iOS a second finger no longer takes back a press** (zgx-desktop): a row or button pressed with one finger clicks as that finger lifts, whatever another finger does, as `UITableView`'s rows and `UIButton` do; two fingers tapping at once click the first's. A thumb resting on the screen used to make taps silently fail. Desktop touchscreens keep a browser's behaviour, where a second finger pans. A thumb resting on nothing a press acts on leaves the press to the next finger, so a tap beside it clicks; one resting on a control keeps it and clicks as it lifts.
