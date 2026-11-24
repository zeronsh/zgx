---
section: performance
---
- **A canvas's changed lines work out their group's place once** (zgx): each changed line of a canvas's text (a terminal's runs) found its group's translation and clip again, walking every ancestor and summing each clipping one's offset. A run of changed lines now shares it, worked out by the first. A log streaming into a terminal takes 1% fewer instructions; nothing else runs new code.
