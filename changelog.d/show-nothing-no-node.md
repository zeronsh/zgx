---
section: performance
---
- **A `ui.show` showing nothing is one node, not two** (zgx, zgx-script): closed with no `otherwise`, it is its region alone, hidden out of layout as the empty box in it was (no flex item, no gap), not a region holding a hidden empty box. A long list of rows, each with a closed show (a tool row's opened part), makes a node fewer a row. `zgx::compose::switch_or_none` is the switch whose cases may show nothing.
