---
section: performance
---
- **A scroll view laid out between device pixels copies its pixels** (zgx-gpu): a view centred in a window of odd width had an edge between pixels, so its every scroll frame repainted the whole view. It now copies within its whole pixels and repaints the sliver past them.
