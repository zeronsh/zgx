**Instruction counts on x86** (cachegrind, exact; CI's perf job on probe PR #1297, v0.1.6 `e077e952` against the v0.1.7 tag `b03245c4`). Luau scenes are the median of 3 heap placements.

| scene | v0.1.6 | v0.1.7 | change |
| --- | ---: | ---: | ---: |
| heavy-both | 1094.0 M | 1092.0 M | -0.19% |
| heavy-stream | 1043.2 M | 1041.4 M | -0.18% |
| heavy-scroll | 76.3 M | 75.9 M | -0.48% |
| heavy-stream-at | 1059.3 M | 1057.4 M | -0.18% |
| lab-stream | 461.3 M | 459.6 M | -0.37% |
| lab-scroll | 67.2 M | 63.9 M | -4.92% |
| animate | 293.0 M | 293.4 M | +0.12% |
| micro-state | 57.0 M | 56.8 M | -0.33% |
| micro-vscroll-fixed | 46.3 M | 46.2 M | -0.25% |
| micro-vscroll-variable | 84.1 M | 84.0 M | -0.16% |
| micro-state-luau | 77.4 M | 77.3 M | -0.21% (median of -0.212%, -0.210%, -0.191%) |
| micro-vscroll-fixed-luau | 61.2 M | 61.1 M | -0.16% (median of -0.162%, -0.161%, -0.127%) |
| micro-vscroll-variable-luau | 125.7 M | 125.7 M | -0.03% (median of -0.029%, -0.047%, +0.007%; layout) |
| mount | 302.5 M | 301.8 M | -0.17% (median of -0.214%, +0.039%, -0.171%; layout) |
| script-update | 1435.4 M | 1339.4 M | -6.69% (median of -6.691%, -6.695%, -6.677%) |
| anime | 3725.8 M | 3728.9 M | +0.08% (median of +0.081%, +0.142%, +0.077%) |
| script-layout | 3018.4 M | 3019.1 M | +0.02% (median of +0.024%, +0.015%, +0.002%) |
| spans-plain | (new) | 49.7 M | |
| spans-follow | (new) | 52.0 M | |
| spans-md | (new) | 58.8 M | |
| list-state | 887.1 M | 887.3 M | +0.00% (median of +0.020%, -0.104%, +0.002%; layout) |
| style | 1327.8 M | 1330.8 M | +0.22% |
| 3d-textured | (new) | 196.3 M | |
| 3d-mixed | (new) | 457.4 M | |
| widgets | 147.8 M | 147.7 M | -0.03% |
| app-mail | 311.4 M | 312.1 M | +0.22% (median of +0.219%, +0.223%, +0.209%) |
| app-chat | 48.3 M | 48.4 M | +0.27% (median of +0.293%, +0.246%, +0.274%) |
| app-dashboard | 177.2 M | 177.2 M | +0.08% (median of +0.052%, +0.196%, +0.077%) |
| app-dashboard-sort | 117.9 M | 118.0 M | +0.13% (median of +0.068%, +0.283%, +0.130%) |
| app-sketch | 45.7 M | 45.9 M | +0.40% (median of +0.431%, +0.403%, +0.336%) |
| app-code | 98.0 M | 97.9 M | -0.02% (median of -0.019%, -0.028%, +0.026%; layout) |
| app-code-search | 86.0 M | 86.0 M | +0.04% (median of +0.044%, +0.038%, +0.104%) |

The cells that rose, by whose code (instructions, v0.1.7 − v0.1.6; the functions are in the job's gate-functions artifact):

| scene | zgx | Luau | C library | other Rust | other |
| --- | ---: | ---: | ---: | ---: | ---: |
| app-sketch | +7,333 | +72,405 | +75,141 | +33,114 | +6,708 |
| app-chat | +53,720 | +25,314 | +48,236 | +13,090 | +1,156 |
| style | +2,957,813 | +0 | +9,957 | +778 | -12 |
| app-mail | +63,115 | +87,740 | +316,511 | +3,528 | +212,201 |
| app-dashboard-sort | +35,684 | +20,640 | +19,672 | +3,873 | +335 |
| animate | +351,729 | +0 | +13,146 | +115 | +0 |
| anime | +2,343,533 | +282,156 | +346,347 | +57,656 | +0 |
| app-dashboard | +63,662 | +24,004 | +1,753 | +1,199 | +1,071 |
| app-code-search | +60,709 | -5,262 | -26,869 | +6,544 | +2,290 |
| script-layout | +291,937 | +736 | +356,671 | +87,831 | +42 |

The x86-only rises match the arm64 run's reading. `style` (+0.22%, all zgx) is inlining that arm64 does not show (−0.05% there). The apps' +0.08 to +0.40% are #1070's z ordering, #1116's percent bases and #1020's editor conversion, followed up in #1276. `animate` and `anime` move by +0.12% and +0.08%. The span and 3D cells are new: v0.1.6 has neither bench.
