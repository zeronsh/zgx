**Instruction counts** (cachegrind, exact) against `b03245c4` (v0.1.7), head `12a539a7` (the 0.1.8 freeze; the tag `50fefc92` adds only version numbers); the gate fails above +100%.

| scene | base | head | change |
| --- | ---: | ---: | ---: |
| heavy-both | 996.4 M | 970.0 M | -2.65% |
| heavy-stream | 948.5 M | 922.5 M | -2.74% |
| heavy-scroll | 74.9 M | 62.4 M | -16.59% |
| heavy-stream-at | 963.9 M | 937.9 M | -2.70% |
| lab-stream | 430.7 M | 432.3 M | +0.37% |
| lab-scroll | 62.9 M | 46.6 M | -25.92% |
| animate | 275.7 M | 264.5 M | -4.07% |
| micro-state | 56.0 M | 35.6 M | -36.51% |
| micro-vscroll-fixed | 45.8 M | 34.1 M | -25.59% |
| micro-vscroll-variable | 81.9 M | 63.2 M | -22.83% |
| micro-state-luau | 76.9 M | 52.7 M | -31.49% (median of -31.488%, -31.400%, -31.513% under 3 heap placements) |
| micro-vscroll-fixed-luau | 60.6 M | 47.9 M | -20.93% (median of -20.942%, -20.930%, -20.914% under 3 heap placements) |
| micro-vscroll-variable-luau | 124.1 M | 102.7 M | -17.26% (median of -17.255%, -17.289%, -17.250% under 3 heap placements) |
| mount | 296.5 M | 292.4 M | -1.59% (median of -1.382%, -1.756%, -1.591% under 3 heap placements) |
| script-update | 1,008.6 M | 968.4 M | -3.98% (like for like: v0.1.8's update_bench and gate_marks overlaid on v0.1.7, mxfast=0 on both sides; gate.py read -27.44%, 1,334.9 M, against v0.1.7's own bench, which counted its data) |
| anime | 2,947.8 M | 2,947.5 M | -0.01% (median of -0.012%, +0.010%, -0.026% under 3 heap placements; layout) |
| script-layout | 2,817.8 M | 2,814.8 M | -0.11% (median of -0.108%, -0.103%, -0.112% under 3 heap placements) |
| spans-plain | 48.5 M | 48.0 M | -1.01% (median of -1.024%, -1.006%, -0.996% under 3 heap placements) |
| spans-follow | 49.7 M | 49.7 M | +0.02% (median of +0.035%, -0.006%, +0.016% under 3 heap placements; layout) |
| spans-md | 55.5 M | 55.2 M | -0.51% (±0.08% with no change) (median of -0.524%, -0.509%, -0.509% under 3 heap placements) |
| list-state | 819.6 M | 819.3 M | -0.04% (median of -0.043%, +0.056%, -0.061% under 3 heap placements; layout) |
| style | 1,246.0 M | 1,246.0 M | -0.00% |
| 3d-textured | 129.7 M | 129.6 M | -0.09% (±0.06% with no change) |
| 3d-mixed | 354.7 M | 346.9 M | -2.18% (±0.08% with no change) |
| widgets | 137.6 M | 137.4 M | -0.09% |
| app-mail | 286.4 M | 286.1 M | -0.12% (median of -0.115%, -0.125%, -0.121% under 3 heap placements) |
| app-chat | 46.5 M | 46.2 M | -0.78% (±0.2% with no change) (median of -0.778%, -0.797%, -0.783% under 3 heap placements) |
| app-dashboard | 167.0 M | 164.4 M | -1.56% (median of -1.558%, -1.571%, -1.549% under 3 heap placements) |
| app-dashboard-sort | 110.3 M | 108.6 M | -1.58% (median of -1.585%, -1.583%, -1.582% under 3 heap placements) |
| app-sketch | 44.5 M | 44.1 M | -0.86% (median of -0.858%, -0.857%, -0.901% under 3 heap placements) |
| app-code | 91.8 M | 88.3 M | -3.79% (median of -3.794%, -3.794%, -3.837% under 3 heap placements) |
| app-code-search | 80.9 M | 79.5 M | -1.82% (median of -1.821%, -1.821%, -1.827% under 3 heap placements) |

<details><summary>Cells that grew, by whose code (instructions, head − base; functions in the job log and its artifact)</summary>

| scene | total | zgx | luau | libc | rust | linker | other |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| lab-stream | +1,582,172 | +202,882 | +0 | +4,497,458 | -9,294,100 | +0 | +6,175,932 |
| spans-follow | +7,159 | +10,649 | -294 | -2,102 | +48 | -2 | -1,140 |

</details>
