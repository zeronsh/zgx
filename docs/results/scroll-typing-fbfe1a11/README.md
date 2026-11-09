# Typing at rest on the experimental prefetched-hybrid pin

Three trials per arm, 20 native posted Unicode keys per trial, actual isolated
Zeron composer at 1114×685 pt, 120 Hz and 2×, frosted material and completed model catalog.
The catalog was verified before and after all nine trials. Each key changed the
scratch composer text and joined a shown paint retaining that text; superseded
input submissions could join the next retaining paint. Every shown update used
tiles at rest. Raw captures and UI copies remain private.

| Arm | Trial p50 range ms | Mean trial p50 ms | Trial p99 range ms | Mean trial p99 ms |
|---|---:|---:|---:|---:|
| Tiles, view clock off | 28.317–30.144 | 29.024 | 42.995–49.747 | 45.632 |
| Tiles, view clock on | 33.679–35.208 | 34.369 | 46.879–50.979 | 48.951 |
| Scroll drawables, view clock on | 33.545–34.248 | 33.869 | 44.604–50.071 | 48.106 |

Engine pin fbfe1a11 includes the closed global view-clock experiment. The
matching view-clock tile control isolates the scroll-drawables flag at rest;
its typing is close to the hybrid here. The separate timer control is about 5 ms
faster at p50. A production hybrid must preserve main’s timer clock at rest and
pass a fresh typing comparison against timer tiles. These numbers do not waive
that requirement or certify scroll cadence.

Timing starts at the native key-post call, not the hardware keyboard timestamp,
and ends at instrumented transaction presentation feedback, not optical scanout.
[Numeric trials](summary.json) retain all three trials per arm.
