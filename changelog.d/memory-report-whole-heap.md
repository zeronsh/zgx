---
section: changed
---
- **`zgx inspect memory` accounts for the whole heap** (zgx, zgx-gpu, zgx-desktop):
  - `text shaping buffers and caches`: what cosmic-text keeps between runs (harfrust's buffer and shape plans, the spans, words and lines it reuses, its font searches' caches).
  - `heap mimalloc resident` and `resident, not in use` (macOS): what footprint charges for mimalloc's pages, and of that what holds nothing live (pages a few blocks keep, size classes' rounding); elsewhere the line says it's unavailable.
  - `devtools effect names` counts the names frames' runs still hold after their effects are gone, and `frame log` each frame's costliest measures; inspected, a chat window's names read 0.6 MB where they said 0.0.
  Each is worked out when a report asks, nothing counted as frames run.
