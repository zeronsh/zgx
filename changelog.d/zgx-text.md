---
section: changed
---
- **Text shaping and measuring build without the renderer** (zgx-text): the fonts, shaped text, prepared lines and `TextServices` zgx-gpu drew text from are a crate of their own, `zgx-text`, with no wgpu or winit. A host that lays text out but never draws (a headless service measuring as a window does) links it alone. zgx-gpu re-exports all of it (`zgx_gpu::text`, `zgx_gpu::prepared`, `zgx_gpu::TextServices`, `zgx_gpu::measure_text`), so nothing that uses zgx-gpu changes.
