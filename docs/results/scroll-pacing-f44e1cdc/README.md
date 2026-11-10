# Marker pacing comparison

> Cadence qualification: the “late” columns in these captures use the prior interval-gap classifier. Correct whole-point rounding can hold a coast position across several refreshes, so those counts are provisional and do not establish missed due moves or ship readiness. Main-thread, GPU, row-build, memory and raw interval measurements remain useful. Native motion and an independent expected-position classifier are being validated; thresholds have not been relaxed retroactively.

**Cadence-scoring qualification (2026-11-09):** historical Wing percentages here used raw wheel-demand windows. On macOS, the recorder runs before the host drops system momentum events; nominal replay classification finds 69 such groups in Wing’s tape. Those ignored deltas must not demand new frames after the app’s own coast stops. Treat these percentages as unfiltered-input statistics, not certified app missed-refresh rates, pending coast-aware scoring. Per-frame measurements and recorded stalls remain evidence; aggregates whose population used raw demand also need qualification. No outliers are removed. Independent continuous native/idle references are unaffected by this input-filtering issue.

**Fixture qualification (2026-11-09):** the benchmark wrapper used a shell fixture whose model catalog did not explicitly set `complete=true`. The actual model-chip loading path uses a 30 fps skeleton, and historical runs did not record or verify catalog completion. Treat the actual-Zeron cells and Zeron idle markers here as loading-state/unverified-settlement measurements, not certification of a settled composer. Paired comparisons used the same fixture condition, so their numbers remain descriptive; late percentage and GPU work may change after settlement. Independent Swift/native references and generic transcript scenes are unaffected. Settled-state ship decisions require fresh matched runs with catalog completion verified.

Canonical engine f44e1cdc plus disclosed timing instrumentation, W 1114×685 pt, 120 Hz, 2×, tiles. All arms use the same binary; baseline has flags unset. This is a changing-marker reference, not a Wing tape replay. Three interleaved pairs per arm, with native brackets around every cell.

| Arm | Role | Scene / gesture / size / present | Trials | Late mean [min,max]; SD % | Interval p99 mean ms | Main >4ms mean [min,max] | Shown mean [min,max] | Paired shown Δ | Gates |
|---|---|---|---:|---|---:|---|---|---:|---|
| idle-asap | base | zeron-idle / reference / W / 0 | 3/3 | 1.76 [0.67,2.31]; 0.94 | 13.89 | 0.00 [0.00,0.00] | 1176.67 [1170.00,1189.00] | 11.00 | fail |
| idle-asap | variant | zeron-idle / reference / W / 0 | 3/3 | 0.79 [0.76,0.84]; 0.05 | 8.33 | 0.00 [0.00,0.00] | 1187.67 [1187.00,1188.00] | 11.00 | fail |
| idle-view-link | base | zeron-idle / reference / W / 0 | 3/3 | 1.64 [1.27,1.87]; 0.33 | 16.67 | 0.00 [0.00,0.00] | 1178.67 [1176.00,1183.00] | 18.33 | fail |
| idle-view-link | variant | zeron-idle / reference / W / 0 | 3/3 | 0.00 [0.00,0.00]; 0.00 | 8.33 | 0.00 [0.00,0.00] | 1197.00 [1197.00,1197.00] | 18.33 | pass |
| idle-lead4000 | base | zeron-idle / reference / W / 0 | 3/3 | 1.33 [0.93,1.70]; 0.39 | 13.89 | 0.00 [0.00,0.00] | 1181.67 [1177.00,1186.00] | -53.33 | fail |
| idle-lead4000 | variant | zeron-idle / reference / W / 0 | 3/3 | 6.19 [3.01,10.94]; 4.19 | 16.67 | 0.33 [0.00,1.00] | 1128.33 [1079.00,1162.00] | -53.33 | fail |
| idle-lead5500 | base | zeron-idle / reference / W / 0 | 3/3 | 1.44 [1.10,2.05]; 0.52 | 16.67 | 0.00 [0.00,0.00] | 1180.33 [1173.00,1185.00] | -472.33 | fail |
| idle-lead5500 | variant | zeron-idle / reference / W / 0 | 3/3 | 69.91 [52.16,80.79]; 15.50 | 16.67 | 0.00 [0.00,0.00] | 708.00 [662.00,786.00] | -472.33 | fail |

LEAD uses separately documented phase validation; original duration-precheck files are preserved. No wait/present comparisons across the6cad tracing change.

View-clock pacing passes the marker gate in all three trials. This does not certify the actual app. ASAP reduces lateness but fails zero late; both LEAD arms lose shown frames and fail.

The original LEAD duration precheck was invalid because CV-link alignment was timed inside `present`, outside `Wait`. Its original reports remain private and unchanged. Phase validation instead checks the intended earlier handover on the observed shown-frame grid, timer-source notes, clock alignment and actual-child flags. The limits are fixed in `scroll_compare.py` and documented in #1809. No timing or outcome was discarded. Wait/present comparisons must not cross the later 6cad8615 tracing fix.

Only conditions and numeric summaries are public. Raw traces, source snapshots, screenshots and logs remain private.
