---
section: fixed
---
- **A cell the pointer crosses between two moves of one turn hears its enter and leave** (zgx-desktop, Linux and Windows): the moves the event loop heard in one turn are handled as one, their last, and a cell crossed between two of them was never entered, so its hover never showed (a hover benchmark missed one cell in 13 runs). The fold stays, for layout and paint; hover now follows the pointer through each folded move (a hit test each, `Ui::try_pointer_passed`), and a turn with one move costs nothing more. macOS hands an app one move a turn itself.
