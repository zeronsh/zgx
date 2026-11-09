# e875c5e0 generic scroll baseline

These 16 single-trial cells are the original-engine baseline, with opt-in
frame-indexed presentation/GPU instrumentation. Conditions: 120 Hz, 2×, opaque,
M=960×720 and L=1440×960; tiles (`p0`) and drawables (`p1`) are separate.
The executable/source audit is inside the archive. [Corrected table](summary.md).

This fixture reproduced 0.85–3.98% total late frames, below Wing's 17% real-app
observation. It cannot certify the actual Zeron app. Every cell fails the
strict row/repaint/headroom/lateness policy. No system floor is established.

`evidence.tar.gz` retains the complete original run records and a separate
reanalysis. The original records used an overly broad `system` label for
under-budget late frames; **that attribution is rejected**. The reanalysis
labels those delays `unattributed`, which fails. Candidate phases and nearby
gesture phase changes are included. The four original timer-heartbeat
reference runs are also **rejected as a system floor**: timer jitter affected
them. They remain in the archive for audit, not as accepted measurements.

Extract with `tar -xzf evidence.tar.gz`. Original evidence is unchanged; the
corrected derivation is in `scroll-baseline-e875c5e0-reanalysis`. This is a
one-trial baseline, not the required three-repeat certification.
