---
section: performance
---
- **A terminal reads a stream of small writes in fewer, larger reads on macOS** (zgx-terminal): a program writing in small pieces (`seq`, a build's log) no longer waits on the tty's lock at every write while the session reads a few bytes at a time. From the second short read in a row, the session's thread lets go of the screen and sleeps 2 µs before reading on. `seq 1 1000000` in a window finished in 0.58 s instead of 0.85 s, with the app's CPU from 1.08 s to 0.52 s. A keystroke's echo and a stream filling the queue (`cat` of a large file) never wait, and both measured as before. Linux reads as it did: its ptys measured slower with any pause.
