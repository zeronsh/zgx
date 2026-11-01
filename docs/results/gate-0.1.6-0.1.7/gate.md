**Instruction counts** (cachegrind, exact) against `e077e952` (v0.1.6), head `fbf1d5e2` (the 0.1.7 freeze; the tag `b03245c4` adds only version numbers); the gate fails above +100%.

| scene | base | head | change |
| --- | ---: | ---: | ---: |
| heavy-both | 997.8 M | 996.1 M | -0.17% |
| heavy-stream | 949.7 M | 948.2 M | -0.16% |
| heavy-scroll | 75.2 M | 74.8 M | -0.48% |
| heavy-stream-at | 965.1 M | 963.6 M | -0.15% |
| lab-stream | 431.2 M | 430.5 M | -0.17% |
| lab-scroll | 66.1 M | 62.9 M | -4.73% |
| animate | 276.2 M | 275.7 M | -0.20% |
| micro-state | 56.1 M | 55.9 M | -0.34% |
| micro-vscroll-fixed | 45.9 M | 45.8 M | -0.26% |
| micro-vscroll-variable | 82.0 M | 81.9 M | -0.07% |
| micro-state-luau | 77.1 M | 76.9 M | -0.23% (median of -0.231%, -0.227%, -0.235% under 3 heap placements) |
| micro-vscroll-fixed-luau | 60.7 M | 60.6 M | -0.17% (median of -0.169%, -0.176%, -0.169% under 3 heap placements) |
| micro-vscroll-variable-luau | 124.2 M | 124.1 M | -0.09% (median of -0.100%, -0.083%, -0.090% under 3 heap placements) |
| mount | 297.1 M | 296.6 M | -0.16% (median of -0.166%, -0.157%, -0.123% under 3 heap placements) |
| script-update | 1,404.6 M | 1,314.1 M | -6.44% (median of -6.443%, -6.439%, -6.448% under 3 heap placements) |
| anime | 2,948.1 M | 2,947.8 M | -0.01% |
| script-layout | 2,817.6 M | 2,817.8 M | +0.01% |
| list-state | 819.5 M | 819.6 M | +0.01% (median of +0.014%, -0.114%, +0.026% under 3 heap placements; layout) |
| style | 1,246.7 M | 1,246.0 M | -0.05% |
| widgets | 137.9 M | 137.6 M | -0.25% |
| app-mail | 286.1 M | 286.4 M | +0.11% (median of +0.109%, +0.125%, +0.101% under 3 heap placements) |
| app-chat | 46.4 M | 46.5 M | +0.30% (±0.2% with no change) (median of +0.347%, +0.299%, +0.212% under 3 heap placements) |
| app-dashboard | 166.9 M | 167.0 M | +0.06% (median of +0.057%, +0.079%, +0.053% under 3 heap placements) |
| app-dashboard-sort | 110.2 M | 110.3 M | +0.10% (median of +0.096%, +0.121%, +0.089% under 3 heap placements) |
| app-sketch | 44.3 M | 44.5 M | +0.40% (median of +0.402%, +0.363%, +0.438% under 3 heap placements) |
| app-code | 91.8 M | 91.8 M | +0.08% (median of +0.059%, +0.079%, +0.078% under 3 heap placements) |
| app-code-search | 80.8 M | 80.9 M | +0.15% (median of +0.150%, +0.151%, +0.136% under 3 heap placements) |

The span (`spans-*`) and 3D (`3d-*`) rows are left out: v0.1.6 has neither bench, and this run reused the previous base's binaries for them. The 3D change, on a textured-only bench overlaid on both sides: the whole bench −56%, a frame 8.81 → 3.25 M instructions (−63%), loading +4.8%.
