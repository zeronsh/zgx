---
section: fixed
---
- **An event loop asked to exit as it starts ends at once on Windows** (zgx-desktop's winit): an exit asked for in `resumed` (which winit delivers as the loop first goes idle) or in `about_to_wait` no longer waits for a window message before ending. Where none came, as on an idle desktop, the loop never returned: `a_turns_pointer_moves_come_as_its_last_on_windows` hung for minutes on a quiet PC while CI's runner, which always had a message coming, passed.
