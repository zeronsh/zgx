---
section: performance
---
- **A window that wakes alone at rest gives its GPU queue back at once** (zgx-desktop): a frame drawn now and then by an idle window (a relative time ticking, an update from elsewhere), with no input since its queue was last given back, gives the queue back at its trim (1.5 s after its last frame) instead of keeping it, and the ~200 MB Metal holds for it, for 10 s. A wake soon after another, or one where input acted, keeps it as before. `ZGX_QUEUE_REST_LONE=0` keeps the old behaviour.
