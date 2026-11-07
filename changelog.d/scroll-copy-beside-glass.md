---
section: performance
---
- **A glass bar over a scroll view no longer repaints the whole view** (zgx, zgx-gpu): what's clear of the bar's blur moves as a copy, and only the band it reaches repaints. A long transcript under a glass composer: 4% fewer instructions over a scroll.
