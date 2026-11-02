---
section: performance
---
- **Only the observed nodes that moved are told so** (zgx): each frame that moved anything gathered every observed node's bounds and visibility (a link's or an icon's in each paragraph) into a list to publish; it now compares each with what was last told and gathers those that changed. An append into a reply with 256 paragraphs costs 7% fewer instructions.
