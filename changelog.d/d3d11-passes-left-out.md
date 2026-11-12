---
section: fixed
---
- With Direct3D 11 (`ZGX_D3D11`), a layer shader with inputs (passes, pictures, GPU views) composites its layer as painted and is named on stderr, instead of drawing nothing (or, reading a GPU view, panicking), until Direct3D 11 runs passes.
