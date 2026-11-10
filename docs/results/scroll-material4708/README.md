# Deferred hybrid: frosted versus opaque material

> Cadence qualification: the “late” columns in these captures use the prior interval-gap classifier. Correct whole-point rounding can hold a coast position across several refreshes, so those counts are provisional and do not establish missed due moves or ship readiness. Main-thread, GPU, row-build, memory and raw interval measurements remain useful. Native motion and an independent expected-position classifier are being validated; thresholds have not been relaxed retroactively.

Three interleaved pairs on M1, real shell, Wing tape, 1114×685 pt, 120 Hz, 2×, settled catalog, reset row 160. Same binary at engine `4708c942`, `ZGX_SCROLL_DRAWABLES=1 ZGX_GPU_STATS=1`, both verified in the actual child. Harness `d3f15c37`. Six valid captures; every cell has a zgx view-clock marker and independent native Metal brackets.

Opaque sets `material.frosted=false`, making `material.see_through()` false; the app covers native glass with solid color. Native window glass configuration itself is unchanged. This is a diagnostic material contrast.

| Material | Late mean [range]; sample SD % | GPU p50 / p99 means ms | Interval p99 trials ms | Main >4 ms trials | Shown trials |
|---|---|---|---|---|---|
| Frosted | 12.54 [10.73, 13.61]; 1.58 | 3.57 / 5.15 | [41.66670900583267, 41.66670796275139, 41.66670799255371] | [6, 2, 4] | [7936, 7966, 7890] |
| Opaque material | 10.93 [9.92, 11.66]; 0.91 | 1.22 / 2.65 | [33.33345800638199, 33.333457976579666, 33.33345800638199] | [7, 4, 2] | [8024, 8144, 8019] |

Opaque minus frosted late differences: −1.63, −0.82, −2.40 percentage points. Shown-count differences: +88, +178, +129. All runs built zero rows in scroll frames. Native brackets were 0–0.17% late; zgx view markers were 0–0.084%. No interval lacked applied-input/coast/position evidence.

GPU cost falls substantially, while opaque scrolls remain about 11% late. Both conditions fail the zero app-attributable or unexplained late-frame ship gate. Neither main-thread work nor blur GPU cost accounts for the remaining scroll-specific gaps.

## Saved opaque interval evidence

The 2,347 late intervals comprise 1,904 finger and 443 coast intervals. These counts match the frozen scorer. Classification gives stationary presented frames precedence, then compares moving frames’ handover and logged host-frame-start intervals.

| Evidence | Finger | Coast |
|---|---:|---:|
| Intermediate shown frame holds position, no applied note | 600 | 102 |
| Holds position, applied target-axis delta zero | 9 | 13 |
| Holds position despite nonzero delta; rounding/cancellation unresolved | 109 | 0 |
| Handovers within 1.5 refreshes, shown interval longer | 26 | 3 |
| Host-frame starts already over 1.5 refreshes apart | 1155 | 317 |
| Host-frame starts timely, handovers farther apart | 5 | 8 |

This is stage evidence, not a proven compositor latch deadline. Deferral notes lack their own timestamp, so absent ticks cannot be distinguished from late deferred arrival. “Nonzero” does not prove rounding alone; cancellation is also possible. The native bracket clears a color each display tick: it bounds presentation, not native input-to-motion. Actual NSScrollView and Zed references are queued separately.

Absolute GPU completion timestamps were unavailable on this pin; duration, wait, exact handover and shown times are retained. No GPU-completion timestamp is inferred from a duration.

## Evidence and reproduction

`scripts/perf/scroll.py run --scenes zeron --gestures wing --sizes W --hz 120 --scale 2 --present 0 --backgrounds translucent --model-catalog settled --reset-row 160 --native-floor --reference-refresh-grid on --rest-seconds 3 --host-env ZGX_SCROLL_DRAWABLES=1 --host-env ZGX_GPU_STATS=1` runs one material cell after supplying the isolated app runner/source and engine source/pin described in `docs/scroll-benchmark.md`. Repeat with `--backgrounds opaque`, interleaving three pairs. Actual-applied/coast telemetry is required; old raw momentum-density scoring is not comparable.

Published files contain reviewed numeric summaries and fixed condition labels only. Raw captures, screenshots, logs, private app source and per-frame note text remain private.
