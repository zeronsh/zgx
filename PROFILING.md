# Profiling a zgx app

`zgx-profiler` measures what a window does each frame and shows it in a HUD,
or writes it to a Chrome trace. It is attached to the music player sample:

```sh
cargo run --release --manifest-path examples/music-player/Cargo.toml -- --library "D:\Music"
```

`--library` takes the folder to read (default: your `Music` folder). Click a
song to play it. It is a test harness for the profiler, not part of the
framework: `examples/music-player/` is in `.gitignore` and is its own Cargo
workspace, so it never enters the framework's build or commit (see
`AGENTS.md`).

## Keys

| Key | What it does |
| --- | --- |
| `F12` | show or hide the HUD |
| `F11` | outline the last frame's damage rects |
| `S` | toggle the spectrum (the scenario the profiler is demonstrated with) |
| `M` | write a marker into the trace |

The keys are bound by `zgx_profiler::keymap()` on the app's root view, and
handled with `Profiler::toggle` and `Profiler::toggle_damage`. Any app can
bind them differently; `attach` only needs a `&mut WindowContext`.

## The HUD

`zgx_profiler::attach(cx, Config::default())` turns on zgx's frame tracing
(`zgx::trace`) and watches the frames the desktop host publishes
(`zgx::diagnostics`). The HUD is a tree of ordinary scene nodes above the
interface, inert, rewritten at most 30 times a second. It shows:

- **Frames per second** of the app's own frames (frames the overlay itself
  caused are counted apart, as `hud`).
- **Frame pacing**: p50, p95 and p99 of the interval between frames, and how
  many frames arrived late (more than 1.5× the median).
- **Phases of the last frame**, as a stacked bar and in microseconds:
  input, script, update, layout, text, damage, paint, encode, submit, present
  and wait. `wait` is time spent waiting for the display, not work.
- **Damage**: how many rects the frame repainted, their area, and
  `F11` outlines them over the window.
- **Reactive work**: signals written, effects run, and the widest cascade (the
  most effects one signal write queued). A signal written more than three
  times in one frame is flagged as thrashing, with its address.
- **Resident memory**, in MiB.

The HUD counts the frames *the app* drew: the overlay's own repaints are
removed from the damage figures, so with the player paused the numbers read
0 fps and near-zero damage instead of measuring the HUD itself. Frames that
paint nothing (`idle`) are counted apart too.

## A trace, in Perfetto

```sh
cargo run --release --manifest-path examples/music-player/Cargo.toml -- --library "D:\Music" --play --seconds 20
```

`--seconds` closes the window after that long and writes `zgx-profile.json`
next to the working directory (`Config::export_chrome_trace`). Open it at
[ui.perfetto.dev](https://ui.perfetto.dev) or `chrome://tracing`:

- One slice per phase per frame (`frame` category), so a frame's split reads
  as a stacked timeline.
- An instant event per frame with its damage rects, node counts, reactive
  work, GPU time where measured, and whether the frame was the overlay's.
- Counter tracks for frame interval, damaged area, signal writes and effects.
- Instant events for markers written with `Profiler::mark` — the sample writes
  `scenario: spectrum on` / `scenario: spectrum off` when you press `S`, so
  two stretches of a capture can be compared by name.

`Profiler::export(path)` writes the same file on demand.

## What the sample shows

Measured on this machine (Windows 10, 1060×700, a 144 Hz display), playing a
12-track library, medians over ~5 seconds:

| Scenario | Damage rects a frame | Damaged area | Frames a second |
| --- | --- | --- | --- |
| playing, spectrum on | 25 | 23 kpx² | ~140 |
| playing, spectrum off (`S`) | 1 | 0.3 kpx² | ~140 |
| paused | 0 | 0 | 0 |

With the spectrum off, the only thing repainting every frame is the elapsed
time next to the scrubber: the static library, the sidebar and the header are
never invalidated. That is fine-grained reactivity doing what it claims — one
signal write (`spectrum`) fans out to 24 bars (the HUD reports the cascade as
×24), and nothing else wakes up.

While paused the window draws nothing at all: the app has no task waiting on
the display (`FrameClock::next`), so the host gets no frames and the process
sits at 0% CPU. Profiling overhead in that state is one relaxed load per
phase.

## Reading the numbers

Retired instructions repeat better than time, but the trace has time. For
what a phase's time is made of, `ZGX_PROFILE=1` still prints per-phase lines
on stderr every 120 frames, and `scripts/perf/` (see
[benchmarking](docs/benchmarking.md)) compares whole scenes by instructions.
This profiler answers the other question: *what did this window do, frame by
frame, and what did the app's own code cost.*

## Checking audio without a window

```sh
cargo run --release --manifest-path examples/music-player/Cargo.toml -- --audio-probe --library "D:\Music"
```

Opens the output device, plays the first track and prints `Sink::get_pos`
every 400 ms: proof that the sink is consuming samples, independent of the UI.

## Notes and limits

- The sample reads tags and lengths of every file before the first frame
  (~50 µs a file). A text that changes after the window is up costs a relayout
  and a redraw; on Windows 10 with an AMD driver this was measured to leave a
  driver thread spinning while the window idles, so the library is read once,
  up front. Spectrum data is decoded on a worker thread and picked up by the
  playback task each frame: nothing polls, so an idle window stays idle.
- Frames the host draws with nothing to paint are counted as `idle` frames,
  not as app frames.
- The HUD's own nodes are part of the frame's node counts (layout/paint in the
  trace) but its damage is removed from the damage figures; the HUD redraws at
  `Config::hud_rate`, 30 Hz by default.
- `Glass` needs Windows 11 22H2 or later for the system backdrop; on Windows 10
  zgx now blurs the desktop behind the window itself (acrylic through the
  composition attribute), which is what this sample relies on for its
  translucent sidebar. The window's own tint is opaque enough to read with or
  without the blur, so losing focus never washes it out.