---
section: performance
---
- **A video player at rest holds no frames** (zgx-video): its scrub cache (up to 512 MB) filled with every frame it played, and kept them once it stopped. It now keeps only frames shown while paused or scrubbing, gives them back when it plays, and gives them back 5 s after nothing more is asked of it. A 1080p clip in an app's window went from about 1 GB of footprint playing and 0.6–0.8 GB at rest to 0.47 GB and 0.2 GB.
