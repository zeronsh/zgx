# Applied-input three-arm scroll comparison, 4708c942

All 12 captures are valid; every arm fails the ship gates. The deferred hybrid
removes most main-thread drawable waiting, but has no proven no-loss result:
its shown-frame difference against the waiting hybrid is +84,−105,−12
(mean −11; sample SD 94.50). Late differences are −2.604,+1.412,−0.841 percentage
points (mean −0.678). Both directions overlap noise. This does not certify
shipping deferral or a zero-late app.

Actual isolated Zeron shell, generated long chat, Wing’s 94.56-second tape,
1114×685 pt, 120 Hz, 2×, frost, row 160, completed catalog verified before/after.
Each hybrid has three interleaved trials and a matched tile control per trial.
Independent native references bracket every zgx marker. Instructions, exact
presentation feedback and actual applied-input/coast/position notes are traced.
The inactive/onset and pixel-limited finger cases remain separate; background
presents cannot hide a held scroll position. Interval p99 includes legitimate
pixel-limited finger gaps; late counts use the documented demand policy.

| Arm | Role | Scene / gesture / size / present | Trials | Late mean [min,max]; SD % | Interval p99 mean ms | Main >4ms mean [min,max] | Shown mean [min,max] | Paired shown Δ | Gates |
|---|---|---|---:|---|---:|---|---|---:|---|
| hybrid-waits | base | zeron / wing / W / 0 | 3/3 | 14.40 [12.62,16.70]; 2.09 | 38.89 | 0.67 [0.00,1.00] | 7829.67 [7678.00,7944.00] | 202.00 | fail |
| hybrid-waits | variant | zeron / wing / W / 0 | 3/3 | 11.77 [10.73,12.32]; 0.90 | 41.67 | 1188.00 [950.00,1384.00] | 8031.67 [8004.00,8056.00] | 202.00 | fail |
| hybrid-defers | base | zeron / wing / W / 0 | 3/3 | 12.69 [11.93,14.15]; 1.26 | 38.89 | 0.67 [0.00,2.00] | 7963.67 [7911.00,8018.00] | 57.00 | fail |
| hybrid-defers | variant | zeron / wing / W / 0 | 3/3 | 11.10 [9.72,12.15]; 1.25 | 38.89 | 3.00 [2.00,4.00] | 8020.67 [7951.00,8119.00] | 57.00 | fail |

| Hybrid arm | Mean acquisition p99 ms | Largest acquisition ms | Mean GPU p99 ms | Largest main work ms | Mean finger/coast late counts |
|---|---:|---:|---:|---:|---:|
| Waiting | 6.826 | 14.673 | 5.412 | 16.099 | 688.67 / 146.67 |
| Deferred | 0.013 | 4.591 | 5.258 | 9.185 | 620.00 / 170.33 |

Every run built zero rows in scroll frames. Deferred/arrival frame counts
match in each trial: 7560/7560,7418/7418,7452/7452. Remaining worst deferred
frames include one 4.591 ms acquisition, one tiles paint and one 6.318 ms paint
with only 0.003 ms acquisition; deferral is not a complete main-time solution.
GPU p99 remains over the 4 ms budget on every arm. Native bracket late rates
are 0–0.167%; the app’s CV-link marker is 6.02–8.12%, its view-link marker
0–0.167%. These are separate references, never an automatic OS-attribution waiver.

Tiles use VIEW_CLOCK=0. Both hybrids use SCROLL_DRAWABLES=1; waiting adds
DRAWABLE_DEFER=0, deferred uses the pin’s default. Thus hybrid-versus-tiles
includes clock and presentation differences; waiting-versus-deferred isolates
the deferral flag. The pin includes the closed global view-clock experiment;
a production PR must preserve timer behavior at rest and repeat typing tests.

[Numeric trials](numeric-trials.json), [aggregate comparisons](comparison.json)
and [numeric phase/latency supplement](phase-supplement.json) contain only
reviewed conditions and numbers. Raw traces, notes, UI/source snapshots, logs
and screenshots remain private. Absolute GPU-completion time was not captured;
the supplement does not reconstruct it from a duration.

Harness [d3f15c37](https://github.com/zeronsh/zgx/tree/d3f15c37), canonical engine
4708c942c80d8b211d1037ac7656a72d0ad4a895, Zeron source
506e1af795452fd1c3a473c4ab16bfa6b6d348b7. To reproduce, fill isolated source
and binary paths in a same-binary manifest and run
`python3 scripts/perf/scroll_compare.py --manifest <manifest.json> --output <new-directory> --trials 3`.
Use the [manual protocol](https://github.com/zeronsh/zgx/blob/d3f15c37/docs/scroll-benchmark.md).
