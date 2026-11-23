---
section: added
---
- **A text area's chips look the same in the browser** (zgx-dom, zgx-script types): an atom's `padding` and `radius` (#2424) set the chip's CSS padding and corners there too, and its `icon` shows before its label (a tinted one, or one in the text's colour, as a mask over that colour; its `bg`, `radius` and `pad` well around it). The icon holds no text, so selection and editing count the label alone. `AtomLook`'s types now have `padding`, `radius` and the icon's well.
