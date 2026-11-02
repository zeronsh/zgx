---
section: fixed
---
- **A runaway script is stopped on time in a backgrounded app** (zgx-script): the watchdog that stops a script past its time budget woke from its timed wait as the system's coalesced timers allowed, and in a process macOS had put in the background (App Nap, a busy CI runner) that came 117 to 197 ms late, the script running on meanwhile. Its waits now end on a strict dispatch timer, which is not coalesced (within 2 ms there), and its thread runs at the interactive QoS; on Linux it takes no timer slack, on Windows the highest thread priority.
