# Settled real-shell Wing tape: tiles and prefetched hybrid

**Cadence-scoring qualification (2026-11-09):** historical Wing percentages here used raw wheel-demand windows. On macOS, the recorder runs before the host drops system momentum events; nominal replay classification finds 69 such groups in Wing’s tape. Those ignored deltas must not demand new frames after the app’s own coast stops. Treat these percentages as unfiltered-input statistics, not certified app missed-refresh rates, pending coast-aware scoring. Per-frame measurements and recorded stalls remain evidence; aggregates whose population used raw demand also need qualification. No outliers are removed. Independent continuous native/idle references are unaffected by this input-filtering issue.

Canonical engine b29b00a78e641e741f05c70c3fa2b4309f18f878 plus matched trace-only instrumentation. Actual Zeron shell fixture, Wing tape, W1114×685 pt,120 Hz,2×,frost. Catalog completion is explicitly true and verified before and after every replay. Three interleaved same-binary pairs; actual child flags, source modes, window geometry and scale verified, with a matched marker and independent native brackets per cell.

Control: `ZGX_VIEW_CLOCK=0`, tiles. Candidate: `ZGX_SCROLL_DRAWABLES=1`, with prefetch enabled by default and the view clock from the closed#1812 pin. This is a combined drawables/view-clock/prefetch comparison. It does not isolate prefetch alone or certify a future PR's typing and clock behavior at rest. On main without#1812, the scroll-drawables flag alone uses timer pacing during scrolls.

| Arm | Role | Scene / gesture / size / present | Trials | Late mean [min,max]; SD % | Interval p99 mean ms | Main >4ms mean [min,max] | Shown mean [min,max] | Paired shown Δ | Gates |
|---|---|---|---:|---|---:|---|---|---:|---|
| prefetched-hybrid | base | zeron / wing / W / 0 | 3/3 | 5.82 [5.26,6.22]; 0.50 | 16.67 | 0.33 [0.00,1.00] | 7822.00 [7781.00,7871.00] | 45.00 | fail |
| prefetched-hybrid | variant | zeron / wing / W / 0 | 3/3 | 3.26 [2.84,3.73]; 0.45 | 22.22 | 488.67 [414.00,558.00] | 7867.00 [7827.00,7893.00] | 45.00 | fail |

Both arms fail the strict ship gates. The candidate shows more frames on average, but still has late frames, main-thread acquisition stalls and GPU p99 above the headroom bound. The1-second acquisition outliers are retained: the engine author identified two concurrent `nextDrawable` waiters as a prefetch bug and supplied a later fix,fc90d0d4. These results measure b29, not that fix.

Main work excludes wait/present but includes drawable acquisition in paint. The numeric supplement reports acquisition separately, all-motion late/on-time timing groups, GPU and phase distributions, and warmed-before/motion-end/3-second-rest RSS. Differences in RSS delta alone are not a leak finding; compare absolute rest values and all three trials. Interval p99 in the table is the mean of per-run p99 values, not a pooled percentile.

All six cells are valid measurements, even though all gates fail. There were no removed outliers or trial replacements in this batch. Earlier real-shell tables without catalog verification remain qualified by their own READMEs. The separate synthetic momentum experiment used a test model that incorrectly demanded movement from OS momentum deltas the macOS host intentionally ignores; it is not used to judge these unmodified Wing-tape results.

Only conditions and numeric summaries are public. Raw traces, logs, source snapshots and window captures remain private. The dist binaries emitted the Luau assertions warning in both arms; these are not claimed to be assertion-free VM builds.
