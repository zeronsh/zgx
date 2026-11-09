# Real Zeron baseline and independent native references

Engine e875c5e0 with opt-in telemetry; actual Zeron native host source
506e1af795452fd1c3a473c4ab16bfa6b6d348b7, isolated fixture geometry and
reference hook. Only `scroll_fixture.luau` omits the disconnected chat publisher
so its timeout cannot erase fixture data. Actual shell/chat rendering remains.
W=1114×685 points, 120 Hz, 2×, effective frost verified true, one trial per path.
Full 94.56-second Wing tape; [headline table](summary.md). No certification claimed.

The synthetic long history uses actual Zeron components: 180 varied turns,
markdown tables/code, folded prompts, local images, tools, sidebar and composer.
It does **not reproduce Wing's observed 17%**: visible late intervals are 5.25%
(tiles) and 3.87% (drawables). Both fail app/GPU lateness, unknown lateness,
row builds, repaints and headroom gates. Further real-history coverage may be
needed; these fixtures must not be represented as Wing's original chat.

Independent Swift/AppKit references bracket each zgx idle cell, with no zgx,
winit or inspect. All four native runs had zero late intervals and zero skipped
drawables. The zgx marker had 14 late intervals (1.18%) on tiles and one (0.083%)
on drawables, with app/GPU under budget. These remain unattributed, not OS noise.
The tile late gaps are irregular (100–2858 ms apart), not a consistent 1 Hz
pattern. See [per-late frame timeline](late-timeline.json). Exact link tick,
handover and absolute GPU completion timestamps were not captured; null fields
mean unavailable. Trace starts/ends are explicitly labeled clock-anchor estimates.

A second independent diagnostic sleeps 2 ms after encoding, before handover.
It also had zero late frames on tiles and drawables. Work includes the sleep;
its main-work maxima are above the default 4 ms headroom gate. This diagnoses
slack and does not waive the headroom requirement. [Numbers](native-delay-2ms.json).

`evidence.tar.gz` preserves complete raw run records, native logs, actual scene
images/tree/state, audit/source snapshots, separate corrected analysis and native
delay logs. Original `late_pct` in raw records used the earlier combined
visible-gap-plus-cancellation formula. **Use the separate analysis headline:**
cancellations are reported independently, so one missed refresh is not counted
twice. The old combined number remains `presentation_incident_pct` for audit.
Two earlier long trials that became a loading skeleton are rejected and are
not included as accepted baseline evidence here.

Run the documented suite with `--scenes zeron --sizes W --gestures wing
--backgrounds translucent --present 0,1 --native-floor`; see
[the measurement contract](../../scroll-benchmark.md).
