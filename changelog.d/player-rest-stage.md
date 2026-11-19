---
section: performance
---
- **A paused video player holds what a gone one does** (zgx-video): left alone, it gives back its stage (the composition's script, interface and renderer) as well as its kept frames, and builds it again when next asked for a frame. Showing the showcase in a 960×540 view, a paused player rests at about 62 MB, the same as with the player dropped, down from about 102 MB; a seek from rest shows in the same 60 to 85 ms.
