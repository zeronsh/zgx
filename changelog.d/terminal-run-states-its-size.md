---
section: performance
---
- **A terminal's runs of text are not measured** (zgx-terminal): each run's size is what the grid gives it, its cells by the line's height, and the scene keeps that instead of shaping the run again for layout and repaint. Measuring cost about a sixth of the terminal's scene work: a log streaming in takes 3.5% fewer instructions, a line at a time 1.5% fewer. A debug build still measures each run and checks that the two sizes agree.
