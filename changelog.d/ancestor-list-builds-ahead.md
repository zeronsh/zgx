---
section: fixed
---
- **A list scrolled by the view around it builds ahead** (`virtualize = "ancestor"`, `virtualize_in_ancestor`): given spare time, it builds the rows a viewport beyond the view before they show, and keeps them as the view scrolls away, as a list that scrolls itself does. It keeps no anchor of its own, so its view moving read as its rows settling: it never left rest, and built only the rows that showed, in the scroll's own frames.
- **A list scrolled by the view around it runs its pass only as the rows it shows change** (`virtualize = "ancestor"`): the boxes around it move on every frame of a scroll or an animation, and each move ran its whole pass again, going through every row it shows. Its pass now follows the rows shown, and while those stay the same (and nothing else it reads changes) it isn't run at all.
