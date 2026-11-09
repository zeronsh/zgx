# Default view clock on the real-shell Wing tape

**Cadence-scoring qualification (2026-11-09):** historical Wing percentages here used raw wheel-demand windows. On macOS, the recorder runs before the host drops system momentum events; nominal replay classification finds 69 such groups in Wing’s tape. Those ignored deltas must not demand new frames after the app’s own coast stops. Treat these percentages as unfiltered-input statistics, not certified app missed-refresh rates, pending coast-aware scoring. Per-frame measurements and recorded stalls remain evidence; aggregates whose population used raw demand also need qualification. No outliers are removed. Independent continuous native/idle references are unaffected by this input-filtering issue.

**Fixture qualification (2026-11-09):** the benchmark wrapper used a shell fixture whose model catalog did not explicitly set `complete=true`. The actual model-chip loading path uses a 30 fps skeleton, and historical runs did not record or verify catalog completion. Treat the actual-Zeron cells and Zeron idle markers here as loading-state/unverified-settlement measurements, not certification of a settled composer. Paired comparisons used the same fixture condition, so their numbers remain descriptive; late percentage and GPU work may change after settlement. Independent Swift/native references and generic transcript scenes are unaffected. Settled-state ship decisions require fresh matched runs with catalog completion verified.

Canonical engine 9ccc903f plus matched trace-only instrumentation, W 1114×685 pt, 120 Hz, 2×, frost, tiles. Same binary: control ZGX_VIEW_CLOCK=0, candidate flags unset. All six cells were valid. Three interleaved pairs, exact row anchor, verified actual-child flags/source modes, with native brackets and a matched zgx marker per cell.

| Arm | Role | Scene / gesture / size / present | Trials | Late mean [min,max]; SD % | Interval p99 mean ms | Main >4ms mean [min,max] | Shown mean [min,max] | Paired shown Δ | Gates |
|---|---|---|---:|---|---:|---|---|---:|---|
| default-view-clock | base | zeron / wing / W / 0 | 3/3 | 8.24 [7.72,8.53]; 0.46 | 25.00 | 71.33 [66.00,76.00] | 7601.67 [7526.00,7685.00] | -98.33 | fail |
| default-view-clock | variant | zeron / wing / W / 0 | 3/3 | 9.20 [7.91,11.33]; 1.85 | 25.00 | 69.33 [69.00,70.00] | 7503.33 [7387.00,7603.00] | -98.33 | fail |

The marker passed under the default, but the actual transcript did not. Mean lateness is higher and all three paired shown-frame deltas are negative. Lateness ranges overlap; the result is not a precise causal effect estimate. No ship gate passes.

A reserved Mac slot paused the batch between pairs 1 and 2. No binary or harness code changed. Subsequent builds finished and cooled before fresh windows/references resumed. The dist binaries emitted the Luau-assertions warning in both arms; this configuration is disclosed rather than described as an assertion-free VM build.

The late-work file joins each delivered late timestamp to its own trace frame within 1 ns. Main time excludes wait/present as in the suite; neighboring-frame maxima and recent expensive-main frames are reported separately. The phase/GPU-wait file is explicitly first-pair analysis; phase is modulo the observed shown grid, not an absolute display-link tick.

Only conditions and numeric summaries are public. Source snapshots, screenshots, raw traces and logs remain private. These results do not certify M/L sizes, 60 Hz, a content-fix pin, or other gestures.
