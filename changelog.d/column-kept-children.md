---
section: performance
---
- **A column growing at its end lays out only its end** (zgx): a column whose height is its children's, laid out again under the constraints it had and its own style unchanged, keeps the places of its earlier children that changed nothing and still hold the result it placed them with, and measures from the first that changed. A reply streaming into a column (its last block growing, or a block appended) no longer asks every block before it again. Every box is where it was; debug builds check each kept child against measuring it.
