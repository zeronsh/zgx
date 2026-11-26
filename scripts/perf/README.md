# Performance tools

Measure in retired instructions, not time: a fixed workload's count repeats
to well under 0.5%, where its time moves by several percent with the
machine's state. Time and GPU numbers come from the windowed comparison
(`scripts/run_macos_comparison.py`) on a quiet desktop.

## A change's cost, in two minutes

```sh
python3 scripts/perf/suite.py                  # every headless scene, this checkout
python3 scripts/perf/suite.py ab origin/main . # main against this checkout's files
python3 scripts/perf/suite.py ab abc123 def456 # two commits
python3 scripts/perf/suite.py --only heavy ab origin/main .
```

The suite runs each headless scene (heavy dashboard in three modes, lab
stream and scroll, animate, script mount, anime, layout, list state, GPU
layout, frame, text, style, widgets; `--all` adds the 10-second state and
core benches) a few times and reports the least disturbed run's
instructions with the spread above it. `ab` builds each revision in a
worktree of its own under `/tmp/zgx-perf` (kept: the next comparison
builds incrementally), interleaves the runs, and marks differences larger
than either side's spread. `suite.py dirs A B` compares two directories of
examples built otherwise from the same source (other `RUSTFLAGS`, a
profile, an allocator). `suite.py clean` removes the worktrees.

A scripted cell that moves on unrelated pull requests, by the same exact
function deltas each time, is a cache hit that depends on when Luau's
collector ran (a weak table deciding it), not their work: find the cache, as
`converted_with_collection` (zgx-script's `state` tests) guards the
converted-table one.

## Heap layout

A change that resizes anything allocated early shifts every allocation after
it. Where those lie changes what the C library costs at the same calls and
bytes: glibc's AVX2 `memcmp` takes a longer path, for lengths 2 to 31, when
`(a | b) & 4095 > 4064` (near a page's end, its own comment says with false
positives), and the allocator splits, merges and consolidates by where its
blocks are. So one 16-byte block kept from before `main`, in a build with no
other difference, moved these cells by this much (instructions, head - base,
CI's runner; zgx's own code moved by +743 or less, or fell):

| cell | 16 or 24 B | 32 B | 816 B | 2448 B | all in |
| --- | ---: | ---: | ---: | ---: | --- |
| app-mail | 0.00% | +0.08% | +0.19% | -0.08% | `memcmp` (up to 569,000) |
| app-dashboard | +0.03% | +0.08% | 0.00% | -0.05% | `memcmp` (129,000) |
| lab-stream | +0.03% | not asked | +0.06% | -0.01% | `memcmp` (257,000) |
| fx-none | +0.23% | not asked | -0.17% | +0.23% | `memcpy`, `malloc_consolidate` (97,000) |
| fx-reader | 0.00% | not asked | +0.27% | +0.03% | `_int_malloc` (282,000) |

(The 32 B arm was the first, placed where only the Luau benches see it; the
rest are in every bench.) One constant that only moves the rest (an unused
string) moved nothing: it is the heap. The response is not monotone in the
size, reaches both signs, and 16 and 24 bytes do the same as both fall in one
allocator chunk. Two builds of one commit (an A/A) read 0 in every cell.

**The rule**, asked of a cell that grew. It is heap layout, and not the
change's cost, when all of these hold:

- zgx's, Luau's and Rust's own instructions, each, grew by 0.02% of the cell
  or less (the split `cgdiff.py` gives). `other`, mostly Luau's collector's
  unattributed frames, moves with the heap as the C library's functions do
  (app-mail's +136,000 on #2193): it is compared by the range and its number
  printed on the row;
- Luau's compiler (`Luau::Parser`, `Compiler`, `Compile::`, `BytecodeBuilder`,
  `DenseHash`) is compared by the range too: its maps are keyed by pointers,
  so where things lie moves it. So are Luau's tables' address-hashed internals
  (`luaH_getp`, `luaH_get`, `mainposition`, `newkey`, `luaO_rawequalKey`),
  but only while what calls them did no more: its interpreter
  (`luau_execute`, `luau_precall`) and the C API (`lua_*`, `luaL_*`, which zgx
  and mlua call) each grew by 0.02% of the cell or less, as each lookup costs
  its caller too. One kept block and no other change moved Luau's compiler by
  -8,800 to +26,800 (video, app-code) and `luaH_getp` by +2,000 to +2,200 in
  every app cell, its interpreter by 357 at most. The rest of Luau, its
  interpreter and collector among it, stays the change's at 0.02%;
- what it asks of the C library is the same: the calls **and** bytes of
  `memcpy`, `memmove`, `memset` and `memcmp` (to 0.02%, and a handful of
  calls as it starts). Equal calls are not enough: a struct that grew copies
  longer at the same calls. Allocations may differ, and the row then says
  how, for the change to explain as its own;
- the rise is inside the base's own range (compared in instructions, the
  gate's change as counted against the base's highest less its lowest, so a
  rise at the edge isn't decided by rounding): the base counted under 8 blocks
  (16 to 3,600 bytes, spread over the page) kept from before it starts, the
  range the cell moves by with no change at all, that day;
- and the rise is 0.3% or less, whatever the range. Past it a person reads
  it.

Anything else stays the change's. The gate reports it beside the functions
it explains (`ZGX_GATE_LAYOUT=0` leaves it out): the widest 8 cells, in at
most 5 minutes, the rest "not shifted". It fails nothing: the gate's
threshold is as it was.

A median over shifts is not the rule: a pure shift doesn't average out over
8 of them. In a model of it (flagged `memcmp` calls under valgrind, head =
base + one kept block, the same 8 shifts on both sides), app-mail's median
stayed at +124,000 instructions for a 16-byte block (-5,000 to +350,000) and
+218,000 for 32 (-69,000 to +427,000), and lab-stream's at +8,000 (16 B) and
-1,000 (32 B): the sum over a whole page is 0, but 8 places of it are not. The base's own spread over the same 8 was 19,000 to 320,000
(app-mail) and 1,000 to 36,000 (lab-stream), which is what a rise is
compared with. Mutants (`test_gate.py`): +0.05% of zgx; 1% more `memcmp`
calls; the same calls copying 1% longer: each is the change's, whatever the
range.

## Where a frame's work goes: `ZGX_PROFILE=1`

Any zgx app or bench run with `ZGX_PROFILE=1` prints, every 120 frames,
what each phase cost per frame:

```text
profile 120 frames: | update 3555.4k instr 191.5 us | layout 3182.2k instr 145.0 us | damage 2285.1k instr 134.2 us | paint 3090.2k instr 324.5 us | encode 737.8k instr 35.0 us | submit 17.5k instr 4.6 us
```

Each phase's numbers leave out the phases it encloses (text shaping during
damage resolution counts as damage). Instructions are the calling thread's.
Unset, a phase boundary costs one relaxed load, and the suite shows no
scene moving (`docs/benchmarking.md`). Building with
`--features zgx/profile-nested` also times styles and text within their
phases (a check in every style and text run, so never in normal builds).
On macOS the phases are `os_signpost` intervals (subsystem `dev.zgx`):
record with Instruments' os_signpost instrument to see them on a timeline
beside a Time Profiler. At startup it also prints a line per milestone
(window created, renderer made, first frame presented, ...) with the
process's instructions and time so far (`zgx::profile::mark`).

## Where a phase's work goes: samples

```sh
scripts/perf/record.sh /tmp/heavy.trace 5 -- target/release/examples/heavy_headless
scripts/perf/record.sh /tmp/app.trace 10 --attach $(pgrep -n my_app)
python3 scripts/perf/samples.py /tmp/heavy.xml --children=render_once
python3 scripts/perf/samples.py /tmp/heavy.xml --under=Host::draw --incl
python3 scripts/perf/samples.py /tmp/app.xml --threads
python3 scripts/perf/lines.py /tmp/heavy.xml target/release/examples/heavy_headless render_flat
python3 scripts/perf/frames.py target/release/examples/heavy_headless zgx::scene
```

`record.sh` wraps xctrace's Time Profiler and exports the samples.
`samples.py` aggregates them by function: self, inclusive, under a
function, its callees, its callers, or per thread. `lines.py` resolves the
samples inside one function to source lines and inlined helpers (build with
`CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`). `frames.py` lists stack
frame sizes, which finds functions paying for large values on the stack.

## Inlining of hot functions

```sh
python3 scripts/perf/inline_scopes.py                       # check, as CI's perf job
python3 scripts/perf/inline_scopes.py --base origin/main    # and name the callees on a failure
python3 scripts/perf/inline_scopes.py record                # write the counts again
```

rustc's MIR inliner gives up on a function past a depth and a budget, so a
change far from a hot function can change what that function inlines, with
no change in its source: #1740 left `Scene::layer_items_walk` 411 of its
520 inlined scopes (+2.8% in fx-layer), and #1824 grew `batches` from 69
lines to 136, and LLVM folded it into `encode_draws`. `inline_scopes.txt`
lists the gate's top functions in zgx, zgx-gpu and zgx-script, each with
its inlined scopes and body lines in release MIR. The check fails on more
than 2 scopes lost, or on growth by both 25% and 30 lines. The counts are
one rustc's (named in the file): under another, the check prints what to
record and passes. A pull request that changes a function's inlining on
purpose records the counts again, so the change is a line in its diff.

## Profile-guided builds

`python3 scripts/perf/pgo.py -p PACKAGE --bin NAME [TRAINING COMMAND ...]`
builds a `dist` binary trained on zgx's headless scenes (and the commands,
`{bin}` being the instrumented binary) into `target/dist-pgo/`.
`docs/getting-started.md` has the recipe for an app outside the repository.
The published comparison builds neither zgx nor GPUI this way.

## The real window path, quickly

```sh
python3 scripts/perf/preflight.py           # is the machine quiet? what runs?
python3 scripts/perf/preflight.py --state   # displays, wallpaper, power mode, build, uptime
python3 scripts/run_macos_comparison.py --scene heavy --quick   # ~10 min, zgx vs GPUI Fast
```

The comparison waits for the preflight, hides Zeron, and records both in
its audit; see `docs/benchmarking.md`.

## Changes A/B'd in windows: `cells.py`

```sh
python3 scripts/perf/cells.py build cells.json   # base + PR heads merged, per cell
python3 scripts/perf/cells.py run cells.json     # first frame after idle, animate, scroll
python3 scripts/perf/cells.py report
```

Cells are candidate changes (and toggles) rather than frameworks. The wheel
tapes the gestures play are in `tapes/`. See "Changes to zgx, A/B on a Mac:
cells" in `docs/benchmarking.md`.

## How finely frame captures resolve pacing

`scripts/framepace.swift` (the comparison's pacing) and frameseq (on the
drawable branch) see a window through ScreenCaptureKit, which captures
only about **110 frames a second on a 120 Hz panel** (home-laptop's M5,
2026-10-26, measured with stamped frames, the same for every present path).
Between two captures a frame can come and go unseen. So at 120 Hz they
cannot tell apart paths whose frame loss differs by 5–10%, and an M5
120 Hz cell must not be ranked on their changed fps or p99 alone.
framepace's stride-2 grid also over-counted a drawable window on the M1
(107 changes a second where the app presented 60).

Worse, a capture changes what it measures: while ScreenCaptureKit captures
a window, macOS skips some of its frames, by an amount that depends on the
Mac and the present path. zgx's tiles skip 12% of frames on the M1 Max
captured and 0.2% not; the M1's tiles at 100 changes a second against 107
for drawables were the capture's doing (tables in `docs/benchmarking.md`,
"Frame pacing: a capture changes it"). A capture says how a window paces
under capture, and stamped frames don't escape it. For an app's own pacing
use the system's feedback (a drawable's presented time, 0 when skipped).

## The window server

`scripts/windowserver_pair.py` and `scripts/windowserver_alternate.py`
measure the window server's CPU for two apps, or two configurations of one,
in paired seconds; see `docs/performance-feature-audit.md`.
