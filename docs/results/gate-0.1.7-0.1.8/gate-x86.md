**Instruction counts on x86** (cachegrind, exact; CI's perf job on probe PR #1346, v0.1.7 `b03245c4` against the v0.1.8 tag `50fefc92`). The base carries v0.1.8's `update_bench` and `gate_marks`, so script-update is like for like. Luau scenes are the median of 3 heap placements.

| scene | v0.1.7 | v0.1.8 | change |
| --- | ---: | ---: | ---: |
| heavy-both | 1092.0 M | 1062.6 M | -2.69% |
| heavy-stream | 1041.4 M | 1012.6 M | -2.77% |
| heavy-scroll | 75.9 M | 65.9 M | -13.12% |
| heavy-stream-at | 1057.5 M | 1028.6 M | -2.73% |
| lab-stream | 459.8 M | 456.9 M | -0.63% |
| lab-scroll | 63.9 M | 49.9 M | -21.95% |
| animate | 293.4 M | 284.4 M | -3.04% |
| micro-state | 56.8 M | 37.8 M | -33.47% |
| micro-vscroll-fixed | 46.2 M | 37.2 M | -19.45% |
| micro-vscroll-variable | 84.0 M | 67.3 M | -19.86% |
| micro-state-luau | 77.3 M | 55.1 M | -28.69% (median of -28.694%, -28.619%, -28.749%) |
| micro-vscroll-fixed-luau | 61.1 M | 51.3 M | -16.07% (median of -16.070%, -16.074%, -16.119%) |
| micro-vscroll-variable-luau | 125.7 M | 106.8 M | -15.04% (median of -15.027%, -15.038%, -15.073%) |
| mount | 301.8 M | 298.6 M | -1.23% (median of -1.067%, -1.420%, -1.228%) |
| script-update | 1041.8 M | 1000.4 M | -4.02% (median of -3.969%, -4.018%, -4.030%) |
| anime | 3728.9 M | 3726.1 M | -0.07% (median of -0.075%, -0.113%, -0.065%) |
| script-layout | 3019.1 M | 3015.3 M | -0.12% (median of -0.127%, -0.098%, -0.116%) |
| spans-plain | 49.7 M | 49.3 M | -0.87% (median of -0.889%, -0.866%, -0.866%) |
| spans-follow | 52.0 M | 52.0 M | -0.09% (median of -0.088%, -0.088%, -0.074%) |
| spans-md | 58.8 M | 58.5 M | -0.54% (median of -0.536%, -0.536%, -0.536%) |
| list-state | 887.3 M | 886.1 M | -0.06% (median of -0.135%, +0.070%, -0.065%; layout) |
| style | 1330.8 M | 1327.8 M | -0.23% |
| 3d-textured | 196.3 M | 196.1 M | -0.13% |
| 3d-mixed | 457.4 M | 449.6 M | -1.69% |
| widgets | 147.7 M | 147.6 M | -0.05% |
| app-mail | 312.1 M | 311.6 M | -0.18% (median of -0.170%, -0.181%, -0.178%) |
| app-chat | 48.4 M | 48.1 M | -0.67% (median of -0.668%, -0.701%, -0.635%) |
| app-dashboard | 177.3 M | 174.6 M | -1.52% (median of -1.525%, -1.565%, -1.517%) |
| app-dashboard-sort | 118.0 M | 116.2 M | -1.53% (median of -1.519%, -1.532%, -1.556%) |
| app-sketch | 45.9 M | 45.5 M | -0.85% (median of -0.846%, -0.847%, -0.721%) |
| app-code | 97.9 M | 93.9 M | -4.10% (median of -4.087%, -4.103%, -4.154%) |
| app-code-search | 86.0 M | 84.4 M | -1.89% (median of -1.889%, -1.894%, -1.939%) |

Every cell falls on x86, lab-stream included (−0.63%; on arm64 it rose +0.37%, from the font cache's check thread and glibc's allocator locks). No cell rose, so there is no split by whose code.
