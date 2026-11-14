---
section: fixed
---
- **An `on_drag` returning nil starts no drag** (zgx-script, zgx): a row can drag only some of what it shows, deciding as each press passes the drag threshold, as a recycled list row must. The press stays a press: no preview and no drag events, and let go on the node it clicks. Before, a nil payload started a drag carrying nil, and every target reading its fields raised an error. A script error in `on_drag` starts none either.
