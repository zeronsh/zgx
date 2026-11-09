# Content fixes and view-clock/hybrid scrolling on the real shell

**Fixture qualification (2026-11-09):** the benchmark wrapper used a shell fixture whose model catalog did not explicitly set `complete=true`. The actual model-chip loading path uses a 30 fps skeleton, and historical runs did not record or verify catalog completion. Treat the actual-Zeron cells and Zeron idle markers here as loading-state/unverified-settlement measurements, not certification of a settled composer. Paired comparisons used the same fixture condition, so their numbers remain descriptive; late percentage and GPU work may change after settlement. Independent Swift/native references and generic transcript scenes are unaffected. Settled-state ship decisions require fresh matched runs with catalog completion verified.

Canonical engine 636b55af131510ada1d0994926aa657356a8079c, with matched trace-only instrumentation. Actual Zeron shell fixture, Wing tape, W 1114×685 pt, 120 Hz, 2× renderer scale, frost. Each arm has three interleaved pairs against the same-binary tiles control with `ZGX_VIEW_CLOCK=0`. The default-clock candidate has flags unset. Hybrid sets `ZGX_SCROLL_DRAWABLES=1`, with the default view clock. Actual child flags and observed frame-source modes were verified; every cell has a zgx marker and independent native brackets.

| Arm | Role | Scene / gesture / size / present | Trials | Late mean [min,max]; SD % | Interval p99 mean ms | Main >4ms mean [min,max] | Shown mean [min,max] | Paired shown Δ | Gates |
|---|---|---|---:|---|---:|---|---|---:|---|
| default-view-clock | base | zeron / wing / W / 0 | 3/3 | 6.41 [5.01,8.36]; 1.74 | 19.44 | 1.00 [0.00,2.00] | 7780.00 [7628.00,7908.00] | -67.00 | fail |
| default-view-clock | variant | zeron / wing / W / 0 | 3/3 | 7.16 [6.26,8.23]; 0.99 | 19.44 | 0.33 [0.00,1.00] | 7713.00 [7609.00,7786.00] | -67.00 | fail |
| hybrid-drawables | base | zeron / wing / W / 0 | 3/3 | 5.49 [4.62,6.44]; 0.91 | 19.44 | 0.33 [0.00,1.00] | 7855.67 [7713.00,7980.00] | 216.33 | fail |
| hybrid-drawables | variant | zeron / wing / W / 0 | 3/3 | 3.05 [2.81,3.39]; 0.30 | 22.22 | 733.00 [628.00,905.00] | 8072.00 [8017.00,8151.00] | 216.33 | fail |

Main work excludes wait and present. Drawable acquisition remains inside paint and is reported separately in the numeric cell data; raw main >4 ms is not a pure CPU-work measurement. Shown-frame counts include the full tape; late intervals use the tape's quantized motion-demand windows. Interval p99 in this table is the mean of per-run p99 values, not a pooled percentile.

One original default-clock pair failed the predefined setup-anchor equality: reply 80 had dy -1 versus 0. Both original captures were retained privately as invalid, and the entire pair was rerun with the original binary, fixture, manifest and harness. Other valid pairs were reused unchanged. The replacement followed a reserved exclusive profiling slot. This exclusion was based on the setup condition, not measured performance.

All strict ship gates remain red. A better mean cadence does not waive late frames, GPU headroom, acquisition waits, or latency. These results do not certify 60 Hz, other sizes/gestures, or the later prefetch pin. The dist binaries emitted the Luau assertions warning in both arms; these are not claimed to be assertion-free VM builds.

Only reviewed conditions and numbers are public. Screenshots, source snapshots, per-frame traces, logs and invalid capture directories remain private.
