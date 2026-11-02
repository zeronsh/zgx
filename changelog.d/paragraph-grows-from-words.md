---
section: performance
---
- **A streamed paragraph is prepared from the words of the one it grew from** (zgx-gpu): each append to a paragraph prepared from its words (#1292) found, looked up and packed every word again. A paragraph that starts with a recent one's text, in the same runs, now keeps that one's words and packed advances up to its last word after a space following a letter or digit. Only the text after it is read, its words looked up, shaped and packed. One word appended to a streaming Markdown reply's 6 KiB paragraph costs 415 K instructions instead of 1.44 M, and 194 K instead of 342 K at 96 bytes. Memory is unchanged, and the peak is lower.
