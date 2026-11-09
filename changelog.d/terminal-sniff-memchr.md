---
section: performance
---
- **The engine-drawn terminal reads its output for escapes a word at a time** (zgx-terminal), not a byte at a time. Streaming a build log costs 1% fewer instructions, and the read for shell-integration commands 77% fewer.
