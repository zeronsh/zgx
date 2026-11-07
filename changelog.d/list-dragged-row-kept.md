---
section: fixed
---
- **A row dragged out of a list is the same node when it's dropped** (zgx): a `ui.list` with heights doesn't build rows folded to nothing, so a row whose place folded away while it was held (as a sidebar's closes up behind it) was taken down, and built again on the drop: its focus, hover and accessible element lost. The row a drag started from now stays built, thin or not, through the drag and its drop; other rows folded away still aren't built.
