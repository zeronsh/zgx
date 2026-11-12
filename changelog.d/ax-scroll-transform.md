---
section: performance
---
- **A scroll sends assistive clients its view's offset, not every row it moved** (zgx-desktop): each scrolled frame that a client listened to walked the whole scene and sent every row with its new place on screen. A scroll view's content offset is now its node's transform, so a step sends that one node, plus the rows that come within reach, which are shaped as before. Clients (VoiceOver, UI Automation, AT-SPI) place every node where it shows, which the tests check across offsets of nested scrolls. A scene with any other transform or zoom still sends places on screen.
