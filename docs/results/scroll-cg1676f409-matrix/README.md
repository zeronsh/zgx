# M1: same-binary scroll presentation comparison

Engine pin: `1676f4093c0da14e51354cdf2c37c2c3fe574c1a`.
Measured 2026-11-10 on the M1 test Mac, 120 Hz, 2x scale, exactly 1114×685 pt.
The actual native chat shell used the settled fixture, frost, reset row 160 and
Wing's tape. Each arm had three interleaved trials with the same guarded HID
CGEvent injector. No rows were built in measured scroll frames.

These **interval-gap counters are diagnostic**, not independently verified due
moves or a ship verdict. Rounded coast holds can legitimately span refreshes.
The expected-position audit is unfinished. Input posting also has a measured
p99 delay of about 4.8 ms, so this is a paired comparison under the same injector,
not a claim that the tape's ideal timing reached either application perfectly.
The AppKit reference holds plain text; this shell has mixed markdown and effects.

| Arm | Raw-valid trials | Gap % mean [min,max] | Main >4 ms mean | Main max ms | GPU p99 ms mean | Acquire p99 / max ms | Shown mean |
|---|---:|---:|---:|---:|---:|---:|---:|
| Tiles | 3 | 18.87 [18.60,19.38] | 3.33 | 7.69 | 5.42 | — | 5718.67 |
| Deferred drawables | 3 | 10.50 [9.80,11.31] | 4.00 | 7.32 | 5.46 | 0.01 / 2.56 | 6111.00 |
| Metal link, 3 drawables | 3 | 10.43 [10.11,10.81] | 13.33 | 6.40 | 5.43 | <0.01 / 0.01 | 6121.67 |
| Metal link, 3, cadence sampling | 3 | 10.09 [9.74,10.62] | 10.33 | 6.67 | 5.41 | <0.01 / 0.01 | 6136.00 |
| Metal link, 2 drawables | 3 | 10.25 [10.02,10.65] | 9.33 | 7.42 | 5.41 | <0.01 / 0.01 | 6114.33 |
| Tiles, cadence sampling | **0** | invalid | — | — | — | — | — |

Main excludes explicit present/wait phases; the maximum is the largest of all
three trials. A trace entry may accumulate several unpresented host updates
before its presentation. Its work totals remain in the report.
All valid arms still exceed the main max and GPU p99 headroom thresholds.

| Arm | Handover→shown p50 ms, mean of trials | Render start→shown p50 ms, mean of trials |
|---|---:|---:|
| Tiles | 18.492 | unavailable |
| Deferred drawables | 22.667 | 23.957 |
| Metal link, 3 | 40.111 | 41.517 |
| Metal link, 3, cadence | 40.114 | 41.520 |
| Metal link, 2 | 40.125 | 41.518 |

The link adds about 17.6 ms against deferred drawables without a clear gap-count
benefit. Fitting away each arm's modal pipeline delay must not conceal that
latency cost. The current link design is held; deferred drawables remains an
experimental candidate, not ship-ready.

## Invalid evidence retained

All three tiles/cadence trials failed the observed half-refresh sample-time
bound: 16, 20 and 18 samples, with maximum ages 14.121, 13.427 and 8.527 ms.
They are excluded. An earlier summary incorrectly changed `invalid` to `fail`
when adding an idle-reference failure. The exporter now checks raw `run.json`
status, and the suite preserves invalid status. The earlier tiles/cadence 18%
comparison is withdrawn.

The initial third tiles setup was also invalid before tape input: its host
refresh estimate was 7.809 ms rather than the requested 120 Hz period. That
attempt remains retained. The valid retry supplies the third tiles trial; no
valid measured outlier was removed.

## Reproduction and evidence

The permanent suite entry point is `scripts/perf/scroll.py`; the physical gate
is manual on a quiet Mac. This comparison additionally used the private test
HID-injector adapter and frozen helper hashes. Its input route differs from
inspect replay, so inspect-only results must not be substituted for these cells.
The next integration step is the validated native reference/input adapter and
expected-position scorer. Raw logs, source staging and window captures stay
private. This directory contains only reviewed summary numbers and conditions.
