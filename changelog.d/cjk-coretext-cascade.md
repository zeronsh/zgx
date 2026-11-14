---
section: fixed
---
- **CJK text is as wide as Chrome lays it out on macOS** (zgx-gpu): a character the run's face lacks draws in the face Core Text gives it as Chrome asks (system-ui's substitute, matched again at Blink's weight), so Han and CJK punctuation take `.PingFang UI SC`'s Display face and kana `.CJK Symbols Fallback SC`, at the text's optical size and with those faces' tracking. Han was 14.29 px at 14 px where Chrome has 13.42, and CJK punctuation and kana took Hiragino or Arial Unicode MS, so Han-heavy lines wrapped a glyph early. Faces the fallback lists can't name (they live in a private framework) are loaded as they're first needed; `ZGX_LOG_FACES=1` prints each character's face.
