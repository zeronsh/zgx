---
section: added
---
- **Compute passes are held to a budget** (zgx-3d, `compute` feature): `World::set_compute_budget` sets the invocations a frame's passes may run (a pass with more alone is refused; passes past it wait their turn) and the GPU milliseconds a pass may take where the device has timestamps (one over it in three of its last eight timed frames is turned off, `ComputeStatus::OverBudget`). `Compute::sandboxed` marks a plugin's pass: its loops bounded so it always ends, a stricter time budget, and refused (`ComputeError::Untimed`) on a device that can't time it.
