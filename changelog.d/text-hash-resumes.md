---
section: performance
---
- **A streamed paragraph's chunk hashes only what it appended** (zgx-gpu): each new state of a growing paragraph, plain or rich (a Markdown reply's), was hashed whole to find it in the text cache, reading the reply so far once a chunk. Once a text is found to grow, the cache keeps the state its bytes left the hash in, and the next state that starts with it is hashed on from there; the same state looked up again is not hashed at all. The hash is the one the whole text makes, bit for bit, and every other text is hashed as before, so scenes that stream nothing pay a comparison a lookup.
