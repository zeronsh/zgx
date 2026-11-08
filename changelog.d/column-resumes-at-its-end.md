---
section: performance
---
- **A column changed only at its end works out only its end** (zgx): a column whose height is its children's keeps its sums (its widest child, its height, where the next child goes) after its last two children, and when only its end changed (a block appended to a streaming reply, or its last block grown) it resumes from them, measuring and placing the end alone rather than going over every child again. Each sum is the same float a whole layout gives; debug builds check them where it resumes.
