---
section: changed
---
- **A traced binding says where in zgx it was made** (zgx): `zgx inspect frames` names a binding `@832 binding compose.rs:6296`, the file and line of the `Ui::bind` call (a prop or feature: a style, a show, a model), beside the node's id and, with the layout inspector on, the script line that made it. Bindings of one handler re-run once, not once a state set; each input event is its own batch.
