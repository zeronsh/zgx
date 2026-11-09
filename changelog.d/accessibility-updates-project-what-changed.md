---
section: performance
---
- **An accessibility update looks only at what changed.** With a screen reader or another assistive client attached, each update used to walk the whole tree. Now text or a move looks at those nodes alone, so what a client adds to a streamed chunk no longer grows with the tree. A flush that changes structure still walks the whole tree, and clients get the same updates as before.
