---
section: performance
---
- **A binding is built where it is kept** (zgx): `Ui::bind` makes its callback's place first and builds the callback in it, where `Rc::new` moved what it holds (a node's styles and inputs, hundreds of bytes) once more, for every styled node a mount makes. The mount benchmark does 0.49% fewer instructions, mounting text 0.34%.
