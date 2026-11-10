---
section: performance
---
- **The image decoding thread gives its heap back once its copies are** (zgx, zgx-desktop): decoded copies are made on that thread and given back by the main one at rest, and mimalloc returns a block another thread freed only when its own thread collects. `zgx::image::set_decoder_rest` names what that thread runs then; zgx-desktop collects its heap. A heavy list on the iOS simulator rested 12 MB lower after a fling; 180 photo decodes on macOS, 2.5 MB lower. Nothing runs as pictures decode or draw.
