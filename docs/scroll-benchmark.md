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
reported as a canceled/skipped submission with its frame attribution. It does
not count as an additional missed refresh: a cancellation can overlap a visible
late interval, or a newer submission can still show on time. `late_pct` counts
actual late visible intervals once; `presentation_incident_pct` preserves the
older combined late-interval-plus-cancellation diagnostic.
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
optional timer/latch `refresh_grid` policy and saves that switch in its
evidence. The CV display-link path can still align its handover independently;
its hold notes remain part of the host floor, not an OS-only reference. A completely still idle window has no
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
tile callbacks also report a clearly labeled trace-clock estimate of render-start-to-shown and frame-end-to-shown. The estimate is calibrated against exact drawable timestamps when available; frame end is not claimed to be the exact Core Animation handover.

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

The physical 120 Hz gate is manual: run `scripts/perf/scroll.py` on a quiet Mac
and attach the table. A ship waits on that report; normal CI does not provide
physical display certification. No self-hosted runner is installed for this
public repository on personal Macs.

The `Scroll performance` workflow template has `workflow_dispatch` as its
**only** trigger: no `pull_request`, `push`, or `schedule`. Its optional
`runs-on: [self-hosted, macOS, zgx-perf-gpu]` job never starts or sits pending
because of a PR. Do not dispatch it without an explicitly arranged runner.
It accepts a pin commit, rates, sizes and render scales, and uploads evidence
even on failure. A hosted macOS VM without a visible 120 Hz GPU desktop cannot
provide this certification.

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
geometry/reference hook and Cargo dependency paths. Only for `scroll_fixture.luau`,
it leaves chat-data publication to the fixture; otherwise a disconnected engine's
timeout replaces fixture state with nil. The real UI/rendering path is unchanged.
A missing transcript during any collection poll invalidates the run. It never installs Zeron.
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

## Independent native reference

`--native-floor` brackets every zgx marker reference with two separate Swift
processes. `scroll_native.swift` imports AppKit/Metal/QuartzCore/IOSurface and
contains no zgx code. NSView's CADisplayLink drives its main run loop at the
requested rate. Drawables clear a full CAMetalLayer; tiles clear a triple-buffered
IOSurface, flip its layer contents in a transaction and use a one-pixel drawable
probe. Both use Metal's presented handlers and the same 1.5-period late rule,
with raw wake/timestamp/target, work, GPU and presentation records. Compilation
finishes before measurement. Native results are evidence about the machine and
probe floor; aggregate similarity never excuses an individual zgx late frame.

A standalone handover-slack diagnostic accepts an optional encoding-to-handover
delay in milliseconds: `target/scroll-native 1114 685 120 tiles 10 2`. Its two-ms
sleep is included in main work. Do not compare that diagnostic to the default
headroom gate without labeling it.

The marker sample itself injects no wheel events, including zero-delta
began/ended events: those can perturb the input/presentation path. Use
`python3 scripts/perf/scroll.py reference --scenes zeron --sizes W
--backgrounds translucent --present 0,1 --repeats 1 --native-floor` to run only
the bracketed marker references. The real-app host must already be built.
At 1× renderer scale, odd logical sizes retain half-point native window
dimensions so the native reference has exactly the same device-pixel extent.

For an instrumented sibling pin, pass `--engine-source CHECKOUT --engine-pin SHA`.
The audit records that checkout's integration commit, working diff and complete
delta from the declared canonical pin, separately from the harness source.
`--host-env ZGX_VIEW_CLOCK=1 --host-env ZGX_SCROLL_LINK=1` enables the explicitly
requested clock experiment on a supporting engine pin. The explicit diagnostic allowlist also accepts `ZGX_SCROLL_DRAWABLES=1`, `ZGX_HANDOVER_ASAP=1`, and `ZGX_LEAD_US=4000` or `5500`; every run records the overrides. With no overrides the
engine's defaults apply. `prepare_zeron.py` uses the existing perf builder's
source-touch step before compiling so a reused target cache cannot supply an
older path-crate artifact whose timestamp looks fresh.

The headless driver mounts with `TestApp::with_spare_time`, drains
`spend_spare_time` after each frame, and enables retained pictures when the
engine provides those APIs. Small Rust trait fallbacks preserve the same driver
on original baseline commits: inherent engine methods take precedence. The
driver records its own main-frame indices before draining spare work, so no
spare entry is charged as a moving frame even on baselines without `Frame.spare`.
This was checked on the old API (3 transcript rows rebuilt over96 frames) and
build-ahead API (zero rows on transcript/plain/grid/nested).

The GPU headless path can close multiple trace entries per logical step; the
driver keeps every entry between spare drains, requires at least96 measured
entries and96 copy/picture frames, and compares partial/full pixels. Source
freshness and trace filtering were checked against530d (3 rows,7 copies),
507531eb (3 rows,96 copies),357def8f (3 rows,96 picture/copy frames) and
aebda8a1 (0 rows,7 copies) on the transcript at2×.
The nested horizontal fixture has repeated visible cells and enough extent for
Wing's whole tape; its inner scroll is tracked while replay targets the visible
outer box. Streaming gestures orbit120 points near the visible tail instead
of demanding frames after clamping against its end. Marker warmup uses no
wheel events, so no gesture grace period can seed a different tick source.

## Interleaved comparisons

Use `scripts/perf/scroll_compare.py --manifest target/comparison.json
--output target/comparison --trials 3` for a same-session engine comparison.
The manifest has a `baseline` descriptor and an `arms` array. Each descriptor
supplies `engine_source`, `engine_pin`, `zeron_source`, `zeron_runner` and an
optional `env` array. Arms additionally name themselves and give a `condition`:

```json
{
  "baseline": {"engine_source": "/checkout/base", "engine_pin": "BASE_SHA",
    "zeron_source": "/checkout/zeron-base", "zeron_runner": "/binaries/base/zeron-zgx"},
  "arms": [{"name": "candidate-tiles", "engine_source": "/checkout/candidate",
    "engine_pin": "CANDIDATE_SHA", "zeron_source": "/checkout/zeron-candidate",
    "zeron_runner": "/binaries/candidate/zeron-zgx", "env": [],
    "condition": {"scene": "zeron", "gesture": "wing", "size": "W",
      "present": "0", "hz": 120, "scale": 2, "background": "translucent"}}]
}
```

Each arm runs baseline then candidate, three times, rotating arm order between
trials. Every cell retains its zgx marker reference and independent native
brackets. `condition.command = "reference"` runs only those short controls.
The harness freezes the manifest and analyzer hash for resumable runs, prevents
idle sleep on macOS, and reports mean, range, sample standard deviation and raw
trial values. A missing or invalid trial cannot certify a cell; every trial's
strict gate must pass independently. Paired late-percentage differences and
shown-frame differences are retained. A negative mean shown-frame difference
is flagged as a regression even if cadence improves. Shown counts cover exactly
the same tape duration; same-refresh submissions, repeated feedback, and a
same-refresh double followed by a skipped refresh have separate counters.

The actual shell starts at row160 through its real `scroll_to_row` API, rather
than a fraction of estimated total height. The temporary UI copy exports that
control without changing installed files. Every baseline/candidate pair must
have the same transcript anchor id and displacement. Height-cache changes can
otherwise put variants at different content. `--reset-row 0` retains the old
fractional-reset protocol solely for explicitly labeled diagnostics.

The native reference starts and stops its sample on CADisplayLink itself.
A missing sample boundary, timed-out process, mismatched display clock or
missing GPU evidence produces an invalid reference saved alongside the raw log.
It cannot disappear from the report or become a passing zero-late control.

Picture-enabled engines can emit `made a picture` and `rastered R px rows`.
`picture_costs` reports count and p50/p99/max main/GPU work separately for
promotion, raster, and composite-only frames. Promotion and raster groups can
overlap. The slowest main frame in each group retains its complete phases and
notes, as does the slowest moving frame for the run. Empty groups mean no
observed marker; they are not reported as zero-cost work.

Motion-window work is independent of renderer notes. The work gate includes
all non-spare frames whose trace interval intersects a requested input-motion
window, plus any other frame with a scroll-path note. This catches row-build
frames that abandon a scroll optimization without emitting its copy/repaint
note. `motion_window_work` reports the window-only population, including
paint/encode/present/wait/rest distributions. Earlier note-only row/main counts
are superseded by reanalysis of the saved traces; presentation late percentages
are unchanged.

Every explicit diagnostic environment override is checked in the actual native
child process. Inherited `ZGX_*` settings are intentionally stripped; use
`--host-env ZGX_GPU_STATS=1` for renderer diagnostics. Missing requested stats
or failed behaviour checks invalidate a run. Pacing probes require the engine's
`frame source:` mode notes, with at least95% in the expected steady mode
(the complete histogram retains transition frames). ASAP and view-link arms
must show no hold, lead arms must hand over at their intended earlier phase
versus their paired baseline, and hybrid presentation must show drawable callbacks during scroll
and tile callbacks at rest. Historical unverified single clock trials are
exploratory and cannot certify an arm.

For a marker **pacing probe**, use `--reference-refresh-grid on` on both baseline
and candidates. This deliberately exercises the latch, so LEAD_US can change
its timer/latch hold. The switch does not disable the separate CV-link align
hold; inspect its notes too. Compare controls in the same batch rather than
crediting a grid switch for a difference from older runs.
Independent native brackets remain separate and contain no zgx pacing policy.

The actual shell also verifies `bench_effective_frost` against the requested background. A setting or platform policy that keeps the effective material opaque invalidates a requested frosted cell rather than silently changing the condition.

### Acquisition and handover timing

Tracing records window `nextDrawable` acquisition and tile GPU waiting separately
from painting. The per-frame handover note and direct drawable submission note
are joined to presented feedback by frame id. Handover-to-display values are
reported only when the clocks and frame bounds agree; missing notes and clock
mismatches are counted, never replaced with zero. The handover timestamp is the
API return; the recorded call duration reconstructs its start. API return can
follow the display timestamp, so return-to-display latency may be negative.

Raw captures, screenshots, source snapshots and build logs remain private.
To export reviewed conditions and numeric results only, run
`python3 scripts/perf/scroll_public.py target/scroll-results/summary.json --output target/scroll-public/summary.json`.
The manual workflow uploads only this allowlisted summary and keeps actual-app
build and replay logs out of its public output. Unknown fields and free-text
notes are omitted. Physical certification remains manual: run
`scripts/perf/scroll.py` on a quiet Mac and attach the table.

The independent Swift native brackets record the same commit/flip-to-shown
latency and acquisition time. Their numeric summaries accompany the app
results. A refresh-phase value is modulo the observed presented-time grid;
it is not an absolute display-link timestamp. Late and on-time frames are
kept separately when interpreting that phase.

LEAD validation uses exact handover timestamps modulo the observed shown-frame
refresh grid, with at least ten clock-aligned samples, at least 90% handover coverage, and
zero detected clock mismatches in each arm. Its median
lead must be within 1 ms of the requested value and move earlier by the intended
amount (also within 1 ms) versus the CV-link baseline's 1.6 ms target. Timer-source
notes are required independently. Wait duration is not a substitute: the timer
and link start at different phases, and older CV-link holds were recorded inside
`present` without a `Wait` span. Phase labels are not absolute link timestamps.

For an engine whose default clock changed, a comparison descriptor can declare
`reference_source` and `scroll_source` (CLI `--reference-source` and
`--scroll-source`) as `view link`, `display link`, or `timer on the grid`.
These expectations are checked against observed notes without forcing the
candidate's default on through an environment flag. Declare the same-binary
control's expected modes separately and adjust expectations for the requested
refresh rate; unsupported or mismatched source notes invalidate the cell.
