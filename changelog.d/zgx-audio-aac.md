---
section: added
---
- **AAC, through the platform's decoder** (zgx-audio, `aac` feature): MP4/M4A and raw ADTS load whole or stream. On macOS that's AudioToolbox, through FFI with no crate added, gapless where the file records its encoder delay and padding (an M4A of 22,050 frames decodes to exactly 22,050). Windows (Media Foundation) and Linux have no decoder here yet: there the feature builds, and an AAC source is an error that says so.
