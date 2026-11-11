---
section: fixed
---
- **A looping stream of a file that decodes to nothing ends rather than hanging every stream** (zgx-audio): a packet that gave no samples (an Ogg's first audio packet does) no longer counts as sound since the loop opened its source again, so a file with one audio packet looped open, empty, for ever on the decode thread: every other stream starved, an offline render waited, and dropping the engine hung. The decode thread now takes at most 256 packets from a stream in a round, a source that gives nothing for 4,096 packets in a row has ended, and dropping the engine waits at most half a second for a decoder that is stuck.
