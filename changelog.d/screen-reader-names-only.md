---
section: performance
---
- **A screen reader hears a name that changes every frame without the tree's geometry sent each frame too** (zgx-desktop): with VoiceOver, Switch Control, Narrator, NVDA or JAWS on, a name or value changing each frame (a timer, a status) updated the whole tree each frame, every moved node's box and text runs with it. A change a reader hears still goes out the frame it happens, alone; geometry keeps its pace (at most every 100 ms), as it does for any client. A sidebar sliding beside a long transcript with a status changing each frame spent 64% less time in the tree's updates.
