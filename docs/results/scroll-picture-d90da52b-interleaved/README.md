# scroll-picture-d90da52b-interleaved

**Fixture qualification (2026-11-09):** the benchmark wrapper used a shell fixture whose model catalog did not explicitly set `complete=true`. The actual model-chip loading path uses a 30 fps skeleton, and historical runs did not record or verify catalog completion. Treat the actual-Zeron cells and Zeron idle markers here as loading-state/unverified-settlement measurements, not certification of a settled composer. Paired comparisons used the same fixture condition, so their numbers remain descriptive; late percentage and GPU work may change after settlement. Independent Swift/native references and generic transcript scenes are unaffected. Settled-state ship decisions require fresh matched runs with catalog completion verified.

| Arm | Role | Scene / gesture / size / present | Trials | Late mean [min,max]; SD % | Interval p99 mean ms | Main >4ms mean [min,max] | Shown mean [min,max] | Paired shown Δ | Gates |
|---|---|---|---:|---|---:|---|---|---:|---|
| clock-p0 | base | zeron / wing / W / 0 | 3/3 | 9.27 [8.73,9.61]; 0.47 | 25.00 | 41.33 [40.00,43.00] | 7466.33 [7413.00,7509.00] | 23.67 | fail |
| clock-p0 | variant | zeron / wing / W / 0 | 3/3 | 8.70 [8.11,9.09]; 0.52 | 25.00 | 90.67 [83.00,100.00] | 7490.00 [7450.00,7511.00] | 23.67 | fail |
| clock-p1 | base | zeron / wing / W / 1 | 3/3 | 4.36 [3.81,5.29]; 0.81 | 25.00 | 380.67 [362.00,408.00] | 7810.67 [7739.00,7854.00] | 18.67 | fail |
| clock-p1 | variant | zeron / wing / W / 1 | 3/3 | 4.58 [4.35,4.79]; 0.22 | 25.00 | 476.00 [457.00,492.00] | 7829.33 [7739.00,7885.00] | 18.67 | fail |

Each arm has three interleaved baseline/candidate trials, identical transcript anchors and native/zgx controls. All raw per-frame data, source/build proofs and callback logs are retained privately on the test Mac. These are failed performance gates, not certification.

Archive SHA256: a221a1614a9f5cf9c8fed0525c30353bc4f73417c96f4415e5751318eb011a03

Correction: original row/main columns above filtered on scroll-path notes and undercounted baseline work. [Motion-window reanalysis](motion-reanalysis.md) supersedes those columns; presentation lateness is unchanged. [Raw corrected analysis](motion-reanalysis.json) includes phase distributions and worst frames.

Raw captures and source/build evidence are private. A verified copy is retained on the test Mac as `evidence-private/scroll-picture-d90da52b-interleaved.evidence.tar.gz`; this path is local and git-excluded. Only summaries, counters and analysis JSON are public.
