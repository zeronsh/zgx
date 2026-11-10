---
section: performance
---
- **A layout reads again only the observed sizes it touched** (zgx): after each layout that changed something, every node whose size is observed (each canvas and icon, `observe_content_size`, `observe_size`) had its content box worked out and compared, whether layout touched it or not. Now only those the layout moved, resized or restyled are read (the scene's queued nodes), with every one read after a registration, after a flush that laid nodes out unseen, where layout touched more than there are, and always those whose padding is a percentage. About 250 instructions a terminal row a frame.
