# Native AppKit coast motion

Numeric motion observations from an actual NSScrollView with NSTextView, 1114 × 685 pt, 2x scale, using phase-bearing native CGEvents. Four synthetic finger lifts have measured last-50-ms speeds about 967, 1559, 3000 and 5838 pt/s. Posting jitter remains visible in the data; this is not ideal event delivery.

`curves.json` separates clip/model position notifications from physically displayed movement, matched from display pixels. Times and positions are relative; displacements and steps use logical points. Each model sample is `[milliseconds after posted lift, relative position, step]`; display samples have the same columns. Input-tail arrays record relative time, delta and posting jitter. The post-lift travel can include the final buffered finger delta, which may land together with the first coast step.

The final 1 pt steps are separated by increasing refresh gaps, including 2–6 refresh holds. That is consistent with whole-point rounding of a quartic; it does not justify a minimum 1 pt step every refresh. The existing zgx curve is being checked against these observations, without changing its constant from one flick.

No late-frame or ship-readiness claim is made here. The earlier interval-gap rule counts some valid quantized holds, and the native capture adapter also needed consecutive duplicate model IDs collapsed. The expected-position scorer is being validated independently. GPU/submit timings are unavailable from this native view. Raw captures, pixels, text and source snapshots remain private; this directory contains only reviewed numeric motion arrays.
