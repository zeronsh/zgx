---
section: added
---
- **The terminal tells programs a cell's size in pixels** (zgx-terminal): `CSI 16 t` is answered with the cell's height and width, the size pictures are drawn by, as `CSI 14 t` already gave the text area's. A program sizing the images it sends by the cell, and asking rather than reading the pty's size (over ssh, where that size may not arrive), heard nothing.
