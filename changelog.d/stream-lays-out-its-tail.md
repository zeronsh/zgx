---
section: performance
---
- **A streamed paragraph prepares, measures and lays out only what each chunk changes** (zgx-gpu): each chunk of a reply streamed into one paragraph took a copy of every word before it, packed them all again to measure, and broke and placed every line again. It now takes the words and lines of the state it grew from, measures from the first line its new words can change, and lays out from there, the lines before kept: a chunk costs the same at 10 lines as at 1,000, laid out bit for bit as the whole paragraph is.
