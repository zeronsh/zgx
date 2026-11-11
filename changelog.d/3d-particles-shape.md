---
section: fixed
---
- **3D particles keep their light however small or long they're drawn** (zgx-3d, `particles` feature): a streak's light is spread along its length, so fast drops are long and faint rather than long white lines; a particle thinner than two pixels is drawn two pixels wide and as much fainter, so distant rain fades instead of vanishing between pixels or flickering; and particles fade out near the camera rather than covering the view.
