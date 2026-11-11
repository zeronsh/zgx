---
section: added
---
- **A bus's level can be read** (zgx-audio). `Bus::set_metered(true)` measures a bus each block, after its gain; master's is measured after its limiter, as it goes out. `Bus::level()` gives the RMS of both sides over about 300 ms and the loudest sample since the last read. A bus not metered costs nothing for it.
