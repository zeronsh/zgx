# Natural editor metrics validation

Linux x86_64, base `ce91055fba63352fa126a44a5714749d94f2380e`.

- `cargo test -p zgx --tests --locked`: 1,386 passed; two ignored.
- `cargo test -p zgx-script --tests --no-fail-fast --locked`: new reactive
  metrics test and all 412 UI-area tests passed. Existing failures remain:
  `monitor_example::sampling_many_processes_holds_steady_memory` and
  `every_test_file_is_listed` (base already omits `list_keys_read` from its areas).
  Monitor was also run in a clean worktree at the base revision: it fails
  there, 1,630 KB after one sample and 6,134 KB after 200 (threshold +2 MB).
- Generated definitions and tracked example definitions were regenerated;
  the definitions consistency checks pass after regeneration.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo fmt --all -- --check`, StyLua 2.6 check: passed.
- `zgx lint` and `zgx check` (luau-lsp 1.70.1) on script and desktop examples:
  125 files, zero warnings/errors each.

Tests cover empty/final lines, Unicode, styled spans/mentions using a
weight-sensitive rich shaper, IME replacement and cursor-only changes,
positive finite widths, reactive publication after layout, viewport resizing,
focus/selection, password rejection, and removal with a retained handle.

The ignored `editor_natural_metrics::natural_metrics_long_draft_cost` experiment
uses 2,001 hard lines, 108,890 bytes, a fallback shaper, and seven repetitions
in an unoptimized build. Median first query after editing: 72,924 us; median
1,000 cached queries: 81 us, with zero shaping calls during cached queries.
This is evidence for the cache and the documented full-content cost, not a
native-font or release-build performance claim. Exact full-document queries
remain opt-in and can be expensive after edits. They never run automatically
on each animation frame; virtualized viewport layout remains independent.

No native editor input behavior was changed. Application-level GPU/frame
validation belongs to the composer integration; these tests do not reproduce
or establish a fix for an intermittent visual glitch.
