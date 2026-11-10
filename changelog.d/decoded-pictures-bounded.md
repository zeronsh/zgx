---
section: changed
---
- **Kept pictures' decoded copies are bounded while a window scrolls** (zgx, zgx-desktop): past sixteen of the window's frames (64 to 192 MiB), those used least recently go as a decode lands. A long scroll through thousands of photos held every copy until the window rested (844 MB after 90 s on an iPhone simulator; 190 MB now), while a scroll through a few hundred decodes no more than before. Nothing runs as an image drops or a picture draws.
