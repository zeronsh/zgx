---
section: performance
---
- **A streamed paragraph's chunk hashes only what it appended** (zgx-gpu): each new state of a growing paragraph, plain or rich (a Markdown reply's), was hashed whole to find it in the text cache, reading the reply so far once a chunk. Once a text is found to grow, the cache keeps the state its bytes left the hash in, and the next state that starts with it is hashed on from there; the same state looked up again is not hashed at all. The hash is the one the whole text makes, bit for bit. Only a text of 256 bytes or more is looked up this way, out of line; every shorter one as before. And the cache's maps take a text's hash as their own instead of hashing it again, so every lookup is a little cheaper.
