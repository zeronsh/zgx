---
section: added
---
- **The engine-drawn terminal shows sixel images** (zgx-terminal), as chafa, lsix and img2sixel write them. They are placed at the cursor over the cells they need, and take those cells from pictures they cover wholly. The cursor then goes to the line of the image's last band, in its column, as xterm, foot and WezTerm put it. Device attributes now announce sixel (`CSI ? 62 ; 4 ; 22 c`, as foot answers), where they said a VT102.
