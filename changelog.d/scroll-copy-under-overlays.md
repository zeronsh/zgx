---
section: performance
---
- **A scroll view copies its pixels under overlays and over a see-through card** (zgx-gpu): something that stays put over a scroll view (a composer floating on a transcript) repaints only its own ink where it is and where the copy put it, and a card of one colour under the view, however see-through, repaints only its rounded corners. The view used to repaint whole. A blurred overlay still repaints with what's behind it.
