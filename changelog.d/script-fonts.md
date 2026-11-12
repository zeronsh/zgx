---
section: added
---
- **`ui.load_font(src)`** (zgx, zgx-script, zgx-gpu, zgx-desktop): a script loads a TrueType or OpenType file (a path from its directory, or a `File` the user picked) and names its family in `font`, where text in a downloaded font (Inter, Geist) used to fall back to another face, measuring and drawing wrong. It reads off the main thread, takes up to 64 MiB, and rejects what is not a font. Every window's text in that family lays out and draws again (`Platform::add_font`, `WindowFactory::add_font`, `GpuContext::add_font`). A plugin needs the `fonts` grant (`Capability::Fonts`), and loads only from its own directory or a file the user picked.
