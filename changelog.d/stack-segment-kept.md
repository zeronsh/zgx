---
section: performance
---
- A deep tree on a small stack (iOS's main thread, 1 MB) mounts and lays out without mapping a stack segment per node: past the low mark, every sibling ran on a segment of its own, mapped, guarded and freed each time. On iOS each thread now keeps one and reuses it; desktop builds are unchanged.
