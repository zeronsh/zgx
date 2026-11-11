---
section: added
---
- **A sound can start partway** (zgx-audio). `Play::offset(frames)` starts it that many of its own frames in. A looping sound starts that far round its loop, and a one-shot started past its end ends at once. A stream decodes its way there, dropping what comes before.
