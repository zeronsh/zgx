---
section: performance
---
- **A streaming Markdown paragraph is mended from where it got to** (zgx-script): while a block streams, the hanging-marker repair (`**bold` shown closed) scanned the whole block into a vector of chars on every append; it now keeps its scan and reads only the bytes appended. An append into a 6 KB paragraph costs 17–20% fewer instructions (13% with 256 paragraphs above it) and its peak heap is up to 23% lower; the repairs are the same, checked against a fresh scan of every prefix.
