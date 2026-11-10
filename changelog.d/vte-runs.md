---
section: performance
---
- **A terminal writes a run of plain text at once** (zgx-terminal): its parser hands each run of printable ASCII over in one call, rather than one a character, and the screen writes the run's cells together (`zgx-vte` 0.15.0, a fork of vte with `Perform::print_str` and `Handler::input_str`, and `zgx-alacritty-terminal` 0.26.4). A frame of plain output costs about 9% less.
