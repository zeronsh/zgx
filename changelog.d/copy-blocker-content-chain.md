---
section: performance
---
- **A scroll frame decides whether to copy its pixels without walking up the tree for each item** (zgx-gpu): what it asks of each paint item in its content, whether the item lies within the scroll view, is answered from the nodes between the view and the item before it, since in paint order an item's parent is one of them. Before, each of the 300 items in a transcript climbed the 16 nodes to the scroll view. Scrolling an expanded 1,000-call tool group in Zeron, `copy_blocker` costs 280K instructions a frame instead of 362K, and paint 2,270K instead of 2,350K (−3.5%), copying the same pixels.
