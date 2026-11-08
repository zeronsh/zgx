---
section: performance
---
- **A flex row that narrows past a sibling's minimum measures its children once** (zgx): a round of flex distribution that stopped one child at its floor shared the room out again, but had already measured the others at the share it gave them, and threw that away. A wrapping text beside a bullet or an icon was measured at three widths a frame while a split was dragged; it is now measured at two. Layout is unchanged: a child whose padding and border are wider than its share still runs the rounds as before.
