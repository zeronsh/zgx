---
section: performance
---
- **Pictures past the image budget upload and decode once, however slowly they decode** (zgx, zgx-gpu): pictures still decoding now count towards the budget's shares, so the others no longer look as if they fit and upload whole, only to go again a frame later. Three 5000×2500 kept files took 13 uploads, three of them 50 MB textures, and 18 to 45 when decoding was slow; they now take 3, at any speed. A decode a larger one has replaced no longer runs, nor is that larger one queued twice.
