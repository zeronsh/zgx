---
section: performance
---
- **A row scrolled in reuses the children lists of the rows scrolled out** (zgx): each container a virtualized list mounted as it scrolled made its children list anew, as the ones its rows scrolled out freed theirs. A container made in a slot a removal freed now takes a removed node's emptied list. Mounting into new slots, as a page's first mount does, is as it was, and the lists kept go when a window comes to rest.
