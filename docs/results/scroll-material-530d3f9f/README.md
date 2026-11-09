# Actual-shell material contrast

Canonical engine530d3f9f, with timing-only instrumentation from#1809. Actual-shell W1114×685pt,120Hz,2×,tiles; three interleaved frost/opaque synthetic flick pairs. Every trial had matching native brackets and a zgx marker. All six trials were valid; all scroll ship gates remain failed. Opaque is a diagnostic appearance.

| Material | Late % mean [min,max]; SD | GPU p50 mean ms | GPU p99 mean ms | WindowServer CPU mean % of one core |
|---|---|---:|---:|---:|
| translucent | 7.016 [5.418,9.070]; 1.868 | 3.512 | 5.074 | 45.04 |
| opaque | 2.618 [2.386,2.870]; 0.242 | 1.122 | 3.479 | 46.07 |

WindowServer CPU uses cumulative process CPU at0.5s intervals on the presentation clock. GPU usage for WindowServer was unavailable without additional privileges. Removing frost reduces app GPU work and lateness, but it leaves late frames. Similar WindowServer CPU in both materials does not establish its cause. Native controls are reported separately.

Only conditions and numeric summaries are published. Raw captures, screenshots, source snapshots, notes and logs remain private. No comparison to a different engine pin or another batch certifies an engine effect.
