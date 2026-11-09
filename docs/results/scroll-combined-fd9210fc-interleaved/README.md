# scroll-combined-fd9210fc-interleaved

**Cadence-scoring qualification (2026-11-09):** historical Wing percentages here used raw wheel-demand windows. On macOS, the recorder runs before the host drops system momentum events; nominal replay classification finds 69 such groups in Wing’s tape. Those ignored deltas must not demand new frames after the app’s own coast stops. Treat these percentages as unfiltered-input statistics, not certified app missed-refresh rates, pending coast-aware scoring. Per-frame measurements and recorded stalls remain evidence; aggregates whose population used raw demand also need qualification. No outliers are removed. Independent continuous native/idle references are unaffected by this input-filtering issue.

**Fixture qualification (2026-11-09):** the benchmark wrapper used a shell fixture whose model catalog did not explicitly set `complete=true`. The actual model-chip loading path uses a 30 fps skeleton, and historical runs did not record or verify catalog completion. Treat the actual-Zeron cells and Zeron idle markers here as loading-state/unverified-settlement measurements, not certification of a settled composer. Paired comparisons used the same fixture condition, so their numbers remain descriptive; late percentage and GPU work may change after settlement. Independent Swift/native references and generic transcript scenes are unaffected. Settled-state ship decisions require fresh matched runs with catalog completion verified.

| Arm | Role | Scene / gesture / size / present | Trials | Late mean [min,max]; SD % | Interval p99 mean ms | Main >4ms mean [min,max] | Shown mean [min,max] | Paired shown Δ | Gates |
|---|---|---|---:|---|---:|---|---|---:|---|
| combined-p0 | base | zeron / wing / W / 0 | 3/3 | 10.43 [9.10,11.96]; 1.44 | 25.00 | 68.00 [64.00,74.00] | 7406.00 [7308.00,7484.00] | 88.00 | fail |
| combined-p0 | variant | zeron / wing / W / 0 | 3/3 | 9.65 [9.28,10.00]; 0.36 | 25.00 | 9.33 [7.00,14.00] | 7494.00 [7453.00,7520.00] | 88.00 | fail |
| combined-p1 | base | zeron / wing / W / 1 | 3/3 | 4.61 [3.98,5.03]; 0.56 | 25.00 | 558.33 [492.00,614.00] | 7840.33 [7806.00,7872.00] | 57.00 | fail |
| combined-p1 | variant | zeron / wing / W / 1 | 3/3 | 4.59 [4.29,5.19]; 0.52 | 25.00 | 277.67 [247.00,324.00] | 7897.33 [7778.00,7976.00] | 57.00 | fail |

Each arm has three interleaved baseline/candidate trials, identical transcript anchors and native/zgx controls. All raw per-frame data, source/build proofs and callback logs are retained privately on the test Mac. These are failed performance gates, not certification.

Archive SHA256: dd07f7b3170512bcde7fe74d5c49f71a177e0c51b900ef910c397ab2ada967f0

Raw captures and source/build evidence are private. A verified copy is retained on the test Mac as `evidence-private/scroll-combined-fd9210fc-interleaved.evidence.tar.gz`; this path is local and git-excluded. Only summaries, counters and analysis JSON are public.
