---
section: changed
---
- **Kept pictures' decoded copies are bounded while a window scrolls** (zgx, zgx-gpu, zgx-desktop): past sixteen of the window's frames (64 to 192 MiB), the copies of pictures still drawn as textures go first, being duplicates, then those used least recently. A long scroll through thousands of photos held every copy until the window rested (844 MB after 90 s on an iPhone simulator; 190 MB now), while a scroll through a few hundred decodes no more than before.
