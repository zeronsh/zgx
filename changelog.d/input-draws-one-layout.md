---
section: performance
---
- **A mount draws its text inputs from one layout** (zgx): each deferred draw laid the scene out again for its own box, so a mount of n inputs ran n layout passes; now it lays out once for all of them, and a draw whose box another moved is drawn again. A layout pass with many boundaries to lay out (one an input) sorts them by one number.
