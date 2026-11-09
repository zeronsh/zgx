---
section: performance
---
- **A long column's paint bound is updated, not recomputed, as its last rows grow**: each chunk of a streamed reply worked out its column's ink over every block it held. A plain container of 64 children or more now keeps the union of all but its last 8, works those out each time, and takes in only those of the rest that grew where they were; anything else is worked out over every child as before.
