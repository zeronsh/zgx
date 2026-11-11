---
section: changed
---
- **An engine holds up to 64 buses, and a bus never added costs nothing** (zgx-audio). The limit was 32. A bus's block buffers and its effect slots are now made when it's added, on the calling thread, not up front for every possible bus. An engine at rest holds 540,722 bytes, down from 566,962, and each bus adds 1,221 bytes when it's added.
