# Expo Exact's heavy list (2026-11-09 to 11-10): zgx against Exact

[Exact](https://github.com/expo/exact) (MIT) ships a benchmark, `bench/heavy-list`, with a spec (`SPEC.md`), a data generator and probes. We wrote its app in zgx's Luau, as a local app kept out of the repository: the same 10,000 messages and 104 JPEGs from its `gen.py` (checksums matched), the same row, and one keyed list of measured rows (`ui.list { measured = true }`). Then we ran Exact's own probes on both apps. Nothing of Exact's code is in zgx.

**Measured** below means a number from these runs. **Inferred** means a reading of them that wasn't measured on its own.

## iOS simulator: before the fixes

An iPhone 17 Pro simulator (iOS 26.3.1, 60 Hz) on an M4 Pro Mac. Exact's in-process probe (`probe/probe.m`) drives each app through its `UIScrollView`; zgx's is the hidden scroller of its iOS host. Five rounds, the apps alternating and the order rotating each round. Exact is at `94b8671` (release). zgx is at `5965811e5`, the iOS host's draft stack, in a release build, with one local probe hook: the scroller is configured once the list exists. Medians, [min–max], n = 5:

| Cell | zgx | Exact |
| --- | ---: | ---: |
| Fling fps | 59.1 [58.5–59.7] | 59.8 [59.8–59.9] |
| Late frames per second | 0.5 [0.2–0.6] | 0.2 [0.1–0.2] |
| Worst frame (ms) | 195.8 [41.2–233.1] | 36.7 [36.2–37.4] |
| Main-thread busy (ms a frame) | 2.5 [2.4–2.7] | 2.8 [2.8–2.8] |
| CPU (ms per second) | 201.9 [189.8–206.0] | 280.1 [277.5–284.6] |
| Main thread CPU (ms per second) | 128.0 [121.2–131.7] | 152.8 [150.5–154.2] |
| Peak footprint (MiB) | 229.3 [228.2–240.1] | 95.2 [91.2–100.4] |
| Live feed (a message in every 250 ms): fps | 59.3 [58.8–59.5] | 60.0 [59.9–60.0] |
| Live feed: worst frame (ms) | 52.0 [48.3–219.4] | 33.3 [16.7–35.7] |
| Live feed: CPU (ms per second) | 223.1 [214.4–225.9] | 318.8 [306.7–323.2] |
| Live feed: peak footprint (MiB) | 285.9 [281.0–287.0] | 85.2 [80.1–86.1] |
| Blank frames in a fling | 0 in every run | 0 in every run |
| Speed ladder: first speed under 55 fps | none to 96,000 pt/s in 4 runs, 6,000 in 1 | 96,000 pt/s in every run |
| Jump to a row: full viewport, p50 (ms) | 35.7 [33.7–40.8] | 34.0 [32.3–35.5] |
| Cold start to a blank-free list (ms) | 449.1 [441.5–455.9] | 304.6 [280.6–310.4] |
| At rest: CPU (ms per second) | 8.7 [7.8–10.4] | 8.3 [8.0–8.6] |

zgx used less CPU and held its frame rate further up the ladder. It lost on worst frame, memory and cold start.

## What each loss came from, and the fixes

| Loss | Cause (measured) | Fix | Measured after |
| --- | --- | --- | --- |
| Worst frame | Taffy laid every flex item out at its min-content width before using the minimum the item named: 3,830 zero-width text measures in one fling, and one Chinese paragraph's frame at 83 ms | [#1831](https://github.com/zeronsh/zgx/pull/1831) (`33189dd92`): an item that names its minimum takes it, and its min-content isn't computed | The fix's test: a row of such items measures its text 24 times, not 42 |
| Worst frame | A Japanese run's simplified Han (车, 见) went to a search of every face: about 400 faces loaded on the simulator, 14 ms on that paragraph's first layout | [#1835](https://github.com/zeronsh/zgx/pull/1835) (`17daf7ea9`): a kana run's Han tries the Chinese faces next, and that search runs once a character; [#1934](https://github.com/zeronsh/zgx/pull/1934) tests it on every platform | The gate's startup scene, which shapes Japanese and Chinese: −21.8% instructions |
| Worst frame, both fixes | | | Single flings on an M5 Max's simulator (not the series' machine, not interleaved): no zgx frame over 16 ms during the fling; each run's worst frame, over the probe's whole run, 34 to 64 ms |
| Cold start | Decoding the feed's JSON into Luau tables: the first frame's script took 2,307 M instructions, 127 ms (headless, M5 Max) | [#1829](https://github.com/zeronsh/zgx/pull/1829) (`6a603ff8f`): JSON is read straight into script values | 1,842 M, 118 ms. `ui.json.load`, which builds tables only as the script reads them, takes it to 289 M, 17 ms; it's [#1833](https://github.com/zeronsh/zgx/pull/1833), not yet merged |
| Peak memory | Decoded copies of pictures kept without a bound while a window scrolls | [#1838](https://github.com/zeronsh/zgx/pull/1838) (`4cc7e594a`): they stay within a budget of 16 window frames, 64 to 192 MiB | Not yet measured on this scene |

Found along the way: inserting rows above a keyed list moved its rows in view ([#1826](https://github.com/zeronsh/zgx/pull/1826), `3af8a2445`, now they stay where they show). The devtools socket's path was too long for the simulator's temporary directory ([#1832](https://github.com/zeronsh/zgx/pull/1832), `7d333aaa6`). And the instruction gate had no cold start: [#1836](https://github.com/zeronsh/zgx/pull/1836) (`d3d411a7f`) adds `startup`, a 1,000-message feed of this shape decoded and laid out, 2% of the gate.

## macOS desktop: before the fixes

An M4 Pro MacBook (macOS 27.0.1, 120 Hz at 2×, on AC), the window 900 × 700 points. The input and observation were both Exact's: its `scrollbench.swift` (trackpad gestures and wheel events posted at the HID tap, frames read with ScreenCaptureKit) and `launchbench.swift`. zgx is at `71a0dfdb1` (main before the fixes, `dist` profile). Each cell is n = 3, the apps alternating; n = 5 for launches.

The models differ here, and the numbers are the app's process only. Exact's AppKit views are composited by the window server, which also moves content in responsive scrolling. zgx draws its own Metal frames. Neither side's window server or GPU cost is counted.

- **Pacing:** zgx showed more changed frames per second in all 10 mode and speed pairs. At 1,000 pt/s the two were close (trackpad 107.0 against 105.2). At 24,000 pt/s zgx showed 104.1 against 86.2 (trackpad) and 103.9 against 79.9 (wheel). It also had fewer gaps over 12.5 ms in every pair.
- **CPU:** zgx's app process used 2.2 to 5.0 times less: 80 against 207 ms per second (trackpad, 1,000 pt/s), 348 against 1,322 (trackpad, 24,000), and 241 against 1,200 (wheel, 24,000). At rest, 0.50 against 1.73 ms per second.
- **Memory:** zgx's peak footprint was higher in every pair (582 against 363 MiB at trackpad 24,000; 308 against 131 at wheel 1,000). Ten seconds after the input stopped the two were level: 153 to 209 MiB for zgx, 128 to 205 MiB for Exact. #1838 bounds the copies behind that peak; it hasn't been measured on this scene.
- **Blank bands:** frames with a band over 250 points without ink. Both had none to 3,000 pt/s. zgx had 19 to 42 per run from 6,000 pt/s (trackpad) and 12,000 (wheel). Exact had 0 to 8. The detector can't tell an unbuilt white row from a photo's grey placeholder (`#E5E5EA`), so the cause isn't established. They came mostly in steady motion, not at reversals.
- **Cold start:** the first window at 271.5 ms for zgx and 268.9 for Exact. Settled content at 354.8 and 318.9. A binary's first launch after it's built or copied also includes macOS checking the new binary before it runs. That was 1.0 to 1.9 s before zgx-run's `main` on an M5 Max, outside either app's code. These cells, near 270 ms, don't include it. A first launch compares only with the other app's first.

## Not compared

- **Windows:** Exact doesn't build on Windows at `94b8671`. Its launch marks are POSIX-only, and its CI has no Windows job.
- **Linux:** left out.

## The rerun, parked

It's built but not run: the same simulator series, with zgx at `5965811e5` plus only #1829, #1831, #1826 and #1835 (tree `3df885f4`). It isn't on the iOS stack's tip, which has moved about 2,500 lines since. Exact is rerun unchanged, as the control for the machine: a zgx cell's before and after count only where Exact's own cell didn't move. The machine has had no quiet half hour since.

What it should show (inferred, not measured): zgx's worst fling frame near Exact's, and its cold start lower by #1829's share. Peak memory should be unchanged, since #1838 isn't in that tree. Once #1833 merges, the cold-start cells get a rerun of their own with the app on `ui.json.load`.
