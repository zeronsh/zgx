---
section: performance
---
- **A column that grows where it stands repaints only what it adds**: a streamed reply's column damaged every block it held each time it grew (a chunk's cost grew with the reply). A plain container that only grows or shrinks in place now damages nothing of its own, and each child whose box changes damages itself.
