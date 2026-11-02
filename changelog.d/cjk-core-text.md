---
section: fixed
---
- **Chinese text on macOS 26 draws in PingFang** (zgx-gpu, zgx-cosmic-text): glyphs whose shapes are only in Apple's `hvgl` table, as macOS 26's PingFang keeps them, are drawn through Core Text into the glyph atlas, at the weight and subpixel position they were shaped at, so Chinese draws in the system's own Chinese face (and at Regular for regular text) rather than falling back to Heiti.
