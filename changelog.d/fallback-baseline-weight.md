---
section: fixed
---
- **A set line height stands on the text font's baseline whatever face a glyph falls back to** (zgx-gpu, zgx-cosmic-text): a fallback glyph's ascent and descent are the strut's own (the default family's face), not the larger of the two, so a line of Hebrew or Arabic, or a mixed line, sits on the row a Latin line of the same style does (SF Hebrew's metrics put it a pixel high before).
- **A fallback face snaps to the named weight CSS matching picks** (zgx-cosmic-text): a variable fallback face (SF Hebrew, SF Arabic) draws 450 in its Medium and 350 in its Light, as Chrome does, instead of at the axis value; the text's own face keeps following the axis. A face's named instances are read from its `fvar`, SF's Medium at 510 standing for 500.
- **`zgx inspect --help` lists `gc` and `reloads`**, and `frames`' `until` and `summary`.
