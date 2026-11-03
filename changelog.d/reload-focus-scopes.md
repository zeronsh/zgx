---
section: fixed
---
- **A reload with a modal open keeps where closing it gives focus back** (zgx, zgx-script): reloading a script while a modal or command palette is open, then closing it (Escape), now gives focus back to what had it before the modal opened, found again in what the reload built, the same way the focused node itself is found (its list row, else its role and name, else its place). Before, focus went nowhere: the modal's scope remembered the node the reload replaced.
