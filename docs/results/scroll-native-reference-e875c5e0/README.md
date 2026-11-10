# Real Zeron baseline and independent native references

> Cadence qualification: the “late” columns in these captures use the prior interval-gap classifier. Correct whole-point rounding can hold a coast position across several refreshes, so those counts are provisional and do not establish missed due moves or ship readiness. Main-thread, GPU, row-build, memory and raw interval measurements remain useful. Native motion and an independent expected-position classifier are being validated; thresholds have not been relaxed retroactively.

**Cadence-scoring qualification (2026-11-09):** historical Wing percentages here used raw wheel-demand windows. On macOS, the recorder runs before the host drops system momentum events; nominal replay classification finds 69 such groups in Wing’s tape. Those ignored deltas must not demand new frames after the app’s own coast stops. Treat these percentages as unfiltered-input statistics, not certified app missed-refresh rates, pending coast-aware scoring. Per-frame measurements and recorded stalls remain evidence; aggregates whose population used raw demand also need qualification. No outliers are removed. Independent continuous native/idle references are unaffected by this input-filtering issue.

**Fixture qualification (2026-11-09):** the benchmark wrapper used a shell fixture whose model catalog did not explicitly set `complete=true`. The actual model-chip loading path uses a 30 fps skeleton, and historical runs did not record or verify catalog completion. Treat the actual-Zeron cells and Zeron idle markers here as loading-state/unverified-settlement measurements, not certification of a settled composer. Paired comparisons used the same fixture condition, so their numbers remain descriptive; late percentage and GPU work may change after settlement. Independent Swift/native references and generic transcript scenes are unaffected. Settled-state ship decisions require fresh matched runs with catalog completion verified.

Engine e875c5e0 with opt-in telemetry; actual Zeron native host source
506e1af795452fd1c3a473c4ab16bfa6b6d348b7, isolated fixture geometry and
reference hook. Only `scroll_fixture.luau` omits the disconnected chat publisher
so its timeout cannot erase fixture data. Actual shell/chat rendering remains.
W=1114×685 points, 120 Hz, 2×, effective frost verified true, one trial per path.
Full 94.56-second Wing tape; [headline table](summary.md). No certification claimed.

The synthetic long history uses actual Zeron components: 180 varied turns,
markdown tables/code, folded prompts, local images, tools, sidebar and composer.
It does **not reproduce Wing's observed 17%**: visible late intervals are 5.25%
(tiles) and 3.87% (drawables). Both fail app/GPU lateness, unknown lateness,
row builds, repaints and headroom gates. Further real-history coverage may be
needed; these fixtures must not be represented as Wing's original chat.

Independent Swift/AppKit references bracket each zgx idle cell, with no zgx,
winit or inspect. All four native runs had zero late intervals and zero skipped
drawables. The zgx marker had 14 late intervals (1.18%) on tiles and one (0.083%)
on drawables, with app/GPU under budget. These remain unattributed, not OS noise.
The tile late gaps are irregular (100–2858 ms apart), not a consistent 1 Hz
pattern. See [per-late frame timeline](late-timeline.json). Exact link tick,
handover and absolute GPU completion timestamps were not captured; null fields
mean unavailable. Trace starts/ends are explicitly labeled clock-anchor estimates.

A second independent diagnostic sleeps 2 ms after encoding, before handover.
It also had zero late frames on tiles and drawables. Work includes the sleep;
its main-work maxima are above the default 4 ms headroom gate. This diagnoses
slack and does not waive the headroom requirement. [Numbers](native-delay-2ms.json).

The privately retained evidence archive preserves complete raw run records, native logs, actual scene
images/tree/state, audit/source snapshots, separate corrected analysis and native
delay logs. Original `late_pct` in raw records used the earlier combined
visible-gap-plus-cancellation formula. **Use the separate analysis headline:**
cancellations are reported independently, so one missed refresh is not counted
twice. The old combined number remains `presentation_incident_pct` for audit.
Two earlier long trials that became a loading skeleton are rejected and are
not included as accepted baseline evidence here.

Run the documented suite with `--scenes zeron --sizes W --gestures wing
--backgrounds translucent --present 0,1 --native-floor`; see
[the measurement contract](../../scroll-benchmark.md).

Raw captures and source/build evidence are private. A verified copy is retained on the test Mac as `evidence-private/scroll-native-reference-e875c5e0.evidence.tar.gz`; this path is local and git-excluded. Only summaries, counters and analysis JSON are public.
