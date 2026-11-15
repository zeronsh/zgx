---
section: performance
---
- **A large subtree taken out repaints what it showed, not the whole window** (zgx): removing a subtree of more than a few hundred nodes at once (`Ui::remove_large`: a page, a long transcript) damages the area it painted, where its parent placed it and within the clips around it, instead of the whole viewport. Switching a page beside a sidebar no longer repaints the sidebar.
