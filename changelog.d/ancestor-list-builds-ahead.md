---
section: fixed
---
- **A list scrolled by the view around it builds ahead** (`virtualize = "ancestor"`, `virtualize_in_ancestor`): with spare time, its rows a viewport beyond the view are built before they show and kept as it scrolls away, as a list that scrolls itself does. Its view moving read as its rows settling (it keeps no anchor of its own), so it never left rest and built only what showed, in the scroll's frames.
- **A list scrolled by the view around it skips passes that change nothing it shows** (`virtualize = "ancestor"`): the boxes around it move with every frame of an animation or a scroll; while the rows it shows stay the same (and nothing else it reads changed), its pass returns at once instead of placing every row it keeps again. A Zeron tool group's scroll frame over 1,000 calls: 7.6M instructions to 6.7M, its spare-time work 8.3M to 6.2M.
