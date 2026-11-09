# e875c5e0 generic scroll baseline

**Cadence-scoring qualification (2026-11-09):** historical Wing percentages here used raw wheel-demand windows. On macOS, the recorder runs before the host drops system momentum events; nominal replay classification finds 69 such groups in Wing’s tape. Those ignored deltas must not demand new frames after the app’s own coast stops. Treat these percentages as unfiltered-input statistics, not certified app missed-refresh rates, pending coast-aware scoring. Per-frame measurements and recorded stalls remain evidence; aggregates whose population used raw demand also need qualification. No outliers are removed. Independent continuous native/idle references are unaffected by this input-filtering issue.

These 16 single-trial cells are the original-engine baseline, with opt-in
frame-indexed presentation/GPU instrumentation. Conditions: 120 Hz, 2×, opaque,
M=960×720 and L=1440×960; tiles (`p0`) and drawables (`p1`) are separate.
The executable/source audit is inside the privately retained archive. [Current visible-interval table](visible-interval-summary.md). The older
[attribution-only correction](summary.md) and privately retained archive retain the combined
gap-plus-cancellation percentage; that is now only a presentation-incident
diagnostic, not the missed-refresh percentage.

This fixture reproduced 0.85–3.98% total late frames, below Wing's 17% real-app
observation. It cannot certify the actual Zeron app. Every cell fails the
strict row/repaint/headroom/lateness policy. No system floor is established.

The privately retained evidence archive retains the complete original run records and a separate
reanalysis. The original records used an overly broad `system` label for
under-budget late frames; **that attribution is rejected**. The reanalysis
labels those delays `unattributed`, which fails. Candidate phases and nearby
gesture phase changes are included. The four original timer-heartbeat
reference runs are also **rejected as a system floor**: timer jitter affected
them. They remain in the archive for audit, not as accepted measurements.

Raw evidence is not distributed in this public repository. Original evidence is unchanged; the
corrected derivation is in `scroll-baseline-e875c5e0-reanalysis`. This is a
one-trial baseline, not the required three-repeat certification.

Raw captures and source/build evidence are private. A verified copy is retained on the test Mac as `evidence-private/scroll-baseline-e875c5e0.evidence.tar.gz`; this path is local and git-excluded. Only summaries, counters and analysis JSON are public.
