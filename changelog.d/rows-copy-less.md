---
section: performance
---
- **A row built again allocates and copies less** (zgx, zgx-script): a view's children are given exactly as many slots as there are (a growing vector reserved four, 2.5 KB, for a container of one), and so are a laid-out box's child placements; a script element's static styles, its children's flow and its wrappers no longer move the 632-byte view in and out on the way; a `ui.each` row's identity is one allocation and its key is compared as it is written; shared state republished builds each changed list or record once, not as a vector copied. `update_bench` rows, whose changed rows are built again, fall 2.1% of their updates (9.6 M instructions in 200), rows updated in place 0.9%.
