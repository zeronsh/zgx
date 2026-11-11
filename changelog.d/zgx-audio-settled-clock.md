---
section: fixed
---
- **A play that starts or resumes a stream is heard when the device says** (zgx-audio). The first block after every start and every resume is untimed: WASAPI fills it before its clock runs. A play in a stream's first blocks is placed from the fourth timed block, counted back, once the clock has settled. Before, a play that resumed a paused WASAPI stream was never timed. A play that started one was placed by a block that reported 2 ms of output latency where the real figure is 42 ms.
