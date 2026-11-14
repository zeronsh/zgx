---
section: fixed
---
- **A wheel tape's notches play at the start of their refresh** (zgx-desktop): `Ui::wheel_tape` played each `lines` notch at its own time, so a tape at 144 Hz landed nearly all of them in the last quarter of a 120 Hz refresh, where macOS never hands an app a wheel's notches, and its frames measured pacing no real wheel meets. Each notch now plays at the start of the refresh it falls due in, on the display link's grid; a tape with the line `# notches as timed` keeps its own times, and a trackpad's motion and the pointer's events are as they were.
