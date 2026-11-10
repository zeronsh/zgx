---
section: fixed
---
- **A video player played to its end tells its last frame before it stops** (zgx-video): behind time, it said it stopped before drawing the last frame, so what listened (the studio, an app's player) could stop on a frame short of the end. It now draws the last frame first, and `Player::listen` delivers a frame before any event that follows it.
