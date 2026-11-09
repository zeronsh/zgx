---
section: performance
---
- **A column streamed into lays out only its end** (zgx): a column whose height is its children's, once its children outgrow their list past sixty-four, looks first at whether only its end changed (its last block grown, or blocks appended): its earlier children's sizes and places from its last result stand, its sums and what it reads resume where its end begins, and only the end is measured. Every other node's layout is as it was. Every box is where a whole layout puts it; debug builds check what it keeps against asking every child.
