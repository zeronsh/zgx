# Native AppKit flick CPU control

Three ordinary NSScrollView/NSTextView windows, 1114×685 pt, 2×, 120 Hz requested. The exact synthetic flick's displacement schedule is applied from CADisplayLink callbacks. All three trials applied 482 events and reached the expected offset without clamping. Active duration was about 4.017 seconds.

WindowServer CPU, sampled from cumulative process CPU every 0.5 seconds on the presentation clock: mean **29.35% of one core**, range [27.88,30.44], sample SD 1.32.

This is a CPU-only control. It has no Metal feedback probe and does not establish the scroll's presented frame rate or lateness. Native continuous Metal clears using the tiles/probe path averaged about 46% in the separate pacing batch; instrumented zgx flicks used 45–46%. The plain AppKit difference cannot by itself establish a full-surface-update cost: instrumentation, presentation path and possibly delivered cadence differ. It is not an interleaved engine effect estimate.

Only conditions and numbers are published. No source snapshots, screenshots or raw captures are included.
