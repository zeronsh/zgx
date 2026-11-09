# Scroll regression gate

The entry point is `scripts/perf/scroll.py`. Its native runs use the same
`crates/zgx-script/examples/transcript_scroll` fixture as the compositor work
(original scene: 9b0adab73, corrected shadow/absolute/text properties). The
fixture adds a deterministic PNG, clipped text and a translucent background
switch. `tables` uses that fixture with a table and code block in every row.
`plain`, `grid` and `nested` use `examples/scroll_scenes`.

## Run

Build shipping optimization, then run the binary without Cargo in the sample:

```sh
cargo build --profile dist --locked -p zgx-run --no-default-features --features inspect,mimalloc
cargo build --profile dist --locked -p zgx-script --features inspect --example scroll_reference
python3 scripts/perf/scroll.py run --scenes transcript,tables --gestures wing,flick,reversals,stream --sizes M,L --hz 120 --scale 2 --output target/scroll-check
```

A failing threshold or unsupported/missing measurement exits nonzero. Every
trial gets its own `run.json`, native log and exact tape; the root has
`audit.json` (commit, source diff, binary hash, platform and arguments),
`summary.json` and `summary.md`. Keep the entire directory. A baseline with
telemetry changes is identified as such by its source diff, not claimed to be
an unmodified binary. The scene's deliberate clipping can issue layout warnings;
script errors and type errors invalidate a run.

Defaults run three trials per scene/gesture/size/background. Use
`--repeats 1 --scenes tables --gestures flick --sizes M --backgrounds opaque`
for a quick reproduction. `--gestures slow` uses subpixel steps;
`flick` and `fling` decay velocity; `reversals` changes direction every quarter
second; `notches` sends line deltas; `stream` changes transcript row 400 near the bottom while
scrolling. `wing` preserves the original 94.56-second tape byte for byte from
[Wing's recording](https://gist.github.com/wing-anara/fa7628315d83138923947bd7a562645a).
It includes its idle gaps and platform-coalesced input. Synthetic dense tapes
also guard against a recorded half-rate stream concealing improvements. The
replay summary now waits for tapes longer than 60 seconds to finish.

Windows are fresh temporary copies, never an installed application or its
persistent data. Each run sets the content size exactly: S=640×480,
M=960×720, L=1440×960 logical points. An inspect mark verifies the actual
size, backing scale and reported display period before sampling. `--scale 1,2`
uses application zoom to test both render scales on a Retina display, changing
the native window size to preserve the logical viewport. `--native-scale`
(default 2) must match the display. This is renderer scale coverage, not a claim
that the panel's physical backing scale changed. `--hz 60,120` verifies each
condition; the runner does not silently emulate a 60 Hz monitor by dropping
frames on a 120 Hz panel. Unsupported rates fail explicitly. `--set-refresh --hz 60,120` selects actual CoreGraphics display modes of the
same logical and physical dimensions, then restores the original mode in a
`finally` block. It compiles the small `scroll_display.swift` helper before
sampling. Unsupported modes are explicit invalid cells, never simulated or
silently skipped; every created window still verifies the reported period.
Without `--set-refresh`, select the actual display mode before running.

Run on an uncovered, visible desktop, on mains power and without builds or
screen capture. ScreenCaptureKit loses frames and changes the path it measures.
`--present 1` measures the drawable path with actual Metal
presentation callbacks. The default `--present 0,1` covers tiles and drawables separately. Tiles use a
one-pixel drawable probe in each flip's transaction (the existing trial probe),
which instruments that path and can have an observer cost. Its callback reports
the transaction's presentation, not an optical pixel measurement. `auto` can
also be requested. Missing feedback always fails, even when CPU frame cadence
looks good.

## Metrics and pass/fail

A refresh period is **1000 / verified requested Hz**, never the observed
median. Thus a display-link latch that halves the rate cannot redefine success.
For intervals during sustained input motion, a frame is late when it appears
more than 1.5 periods after the previous one. A zero Metal `presentedTime` is
counted as a skipped drawable and included in the late numerator and denominator.
Every callback carries the exact devtools frame index, including skipped ones.
Metal command-buffer completion callbacks also record GPU time against that
same index. The analyzer uses those completed timestamps; the legacy inspect
GPU field is retained as `trace_gpu_micros`, because it can land in the following
trace entry. Callback records are bounded by the sampled frame indices, so
frames submitted after the sample cannot become unaccounted-for late frames.
The report preserves all raw presented times and frame-completion times.

Idle tape gaps, isolated notches, and slow steps that accumulate less than one
device pixel do not require a new picture every refresh. The analyzer derives
motion windows from the tape, accumulates fractional device-pixel motion and
counts long intervals only inside dense moving input. It reports idle intervals
separately. These are **input-demand windows**, not a claim of optical pixel
tracking or OS hardware-event latency. Autonomous momentum after the tape's
moving-input region is retained in raw feedback but excluded from the late gate. For
pixel-limited `slow` and isolated `notches`, a 120 Hz interval distribution is
not meaningful: sparse cells instead require at least 80% of the planned
quantized pixel updates to appear, ten traced moving frames, and all the same
work/GPU/row/repaint and late-attribution checks. Idle gaps remain separate.

The target is **zero late frames attributable to the app or GPU**. Every late
or skipped presentation cites its frame record(s) in `late_attributions`:

- `app`: app main-thread work exceeds the 4 ms budget;
- `gpu`: GPU work exceeds its budget;
- `app_pacing`: engine notes explicitly report holding an extra refresh;
- `unattributed`: all remaining delays, including under-budget app/GPU work
  followed by a late presentation. These fail until independent display-clock
  evidence proves a system delay. Missing frame identity or GPU timing also fails.
- `system`: reserved for independently proven system delays; the current
  automatic classifier never assigns this label.

This is attribution from engine records, not an OS scheduler trace. Under-budget work alone does not prove a system delay: input pacing, the
display-link latch and presentation waits can still be app defects. The report
keeps exact candidate frames, summed phase times, notes and phase changes within
50 ms for investigation. It does not
subtract a guessed system percentage or excuse costly app frames.

The stable headroom gates are app-work **p99 and max ≤4 ms**, GPU **p99 <4 ms**
(under half a refresh at 120 Hz), **zero rows built in warmed scroll frames**,
and **zero full scroll-view repaints**: copying pixels or compositing a retained
picture must carry every warmed move. Any app/GPU-attributed late frame fails;
any unattributed late frame fails. Total late percentage remains in the report,
with app/GPU and system counts separately. The absolute main-thread instruction
ceiling is 50 million per frame, backed by the deterministic cachegrind gate.
At least 30 active presentation intervals, ten traced frames, GPU timings and
an instruction counter are required. Missing evidence never passes, and all
three repeats must pass. CLI overrides are explicit diagnostic policies saved
in the audit; the CI workflow uses these defaults.

Before each distinct size/refresh/scale/background/present-mode condition, a
10-second **idle reference** presents only a changing 8×8 marker on an otherwise
static window. Its tiny host (`scroll_reference`) advances the marker through
`cx.frames.next()` on the actual display clock. A timer-driven heartbeat would
add timer jitter to the supposed system floor. The reference disables the
optional `refresh_grid` cadence-holding policy, which the real scene runs still
test, and saves that switch in its evidence. A completely still idle window has no
presentations and cannot establish a floor. The reference uses the same host,
presentation path and telemetry. Its late frames remain unattributed and are reported
separately and linked from each matched scene run. If the reference fails its
app/GPU/headroom or attribution checks, those scene runs cannot pass. This
measures the minimal host/compositor floor for that session, not an excuse to
ignore an app-work or GPU budget violation.

Per run, the suite also reports p50/p99/max presentation interval; frames whose
app main-thread phases total over 4 ms (excluding `wait` and `present`); GPU
p99; instruction p50/p99/max; copied/repainted frame share and exact blocker
notes; row construction inside scroll frames; and process RSS before/after.
RSS excludes GPU allocations and is in KiB. Main work with and without
`present`, plus `present` and `wait` distributions, are reported separately;
these waits are not automatically attributed to the system; retired instructions provide the deterministic CPU gate.
Row construction is an engine trace note, independent of script naming.
Build-ahead `spare: true` entries are excluded from scroll-frame headroom and
row-build counts, with spare-frame and prefetched-row totals reported separately.
They remain in per-late-frame attribution: spare app work that overruns its
budget cannot be passed off as a system wait.
Retained picture frames (`composited its picture`) are counted as scroll frames
and reported separately from pixel copies. Drawable runs additionally gate
render-start-to-shown p99 at 25 ms (`--max-present-latency-ms`), so queued but
smooth frames cannot pass. This is render latency, not hardware input latency;
tile-probe callbacks do not currently carry a render-start timestamp.

The runner drains `frames n=400 since=<last-read-index> runs=0` every second
and records a new mark. The cursor remains the last frame **read**, so a frame
between the read and mark cannot disappear. A full 400-frame batch is rejected
as possible overflow. The inspect server permits reads while replay is pending.

## Normal CI and GPU correctness

```sh
python3 scripts/perf/test_scroll.py -v
python3 scripts/perf/scroll.py headless --strict
GPU=1 python3 scripts/perf/scroll.py headless --strict
```

The headless example visits and reverses over the same region twice, then
measures a third traversal through the public scroll path. Strict mode fails
if a warmed row rebuilds. With `GPU=1`, it also requires a fast path on all 96 warmed moves and compares each partial render against a full repaint to catch stale
pixels. These checks intentionally expose the pinned baseline's row rebuilding
and table/fade copy blockers; the fixes must land before this gate is green.
GPU checks run in normal Linux CI with its software adapter. They are correctness
checks, not native timing claims. Normal CI also tests the analyzer against
half-rate pacing, idle gaps, quantized motion, skipped and missing feedback.

`scripts/perf/gate.py` and `suite.py` include four instruction scenes:
`scroll-transcript`, `scroll-plain`, `scroll-grid`, `scroll-nested`. The existing
cachegrind A/B gate supplies deterministic instructions on Linux. The scroll
binary is built separately with `inspect`, so tracing does not change the
features of existing benchmarks. macOS phase counters supply the absolute
per-frame ceiling; zero/unsupported hardware counters do not supply a native pass.

The `Scroll performance` workflow runs the native suite on a self-hosted Mac
labelled `zgx-perf-gpu`. It accepts a pin commit, rates, sizes and render scales,
and uploads the complete evidence even on failure. A hosted macOS VM without
a visible 120 Hz GPU desktop cannot provide this certification.

For a sibling fix, check out or cherry-pick its engine commit into a separate
worktree, retain this suite and the exact fixture/tapes, rebuild, and repeat the
same command. Compare runs from the same quiet desktop session, dimensions,
scale, refresh and presentation mode. Preserve each binary hash and source diff.
Do not merge unrelated baseline and fix trials or substitute a timing result
from another machine.

## Actual Zeron headline scene

The `zeron` scene runs the actual `zeronsh/zeron` `apps/zgx` native host and
its real shell/chat components, using `shell_fixture.luau`. The benchmark
wrapper supplies 180 varied turns with wide markdown tables, code, local images,
folded user prompts, tools, 400 sidebar sessions and the frosted composer.
`W` is Wing's exact **1114×685 point** content window. This real-app result
is the headline; generic transcript results cannot certify the real app.

Use a dedicated source clone; `prepare_zeron.py` changes that clone's fixture
geometry/reference hook and Cargo dependency paths. It never installs Zeron.
Each launch strips inherited Zeron settings and uses fresh scratch `HOME`,
`ZERON_DATA_DIR`, UI files and an unused engine port; engine spawning is disabled.
No installed app or `~/.zeron` is used. Build proofs retain source diffs, lockfile
and executable hashes. The matched reference uses the same real shell and host,
with an 8×8 marker advanced by its native display clock.

```sh
python3 scripts/perf/prepare_zeron.py \
  --source ~/Documents/GitHub/zeron-scrollbench \
  --zgx "$PWD" --output target/zeron-native/current
python3 scripts/perf/scroll.py run --scenes zeron --gestures wing,flick,stream \
  --sizes W,M,L --backgrounds translucent --hz 120 --scale 2 \
  --zeron-runner target/zeron-native/current/zeron-zgx
```
