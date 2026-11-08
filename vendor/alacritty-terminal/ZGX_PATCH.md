# zgx patch to alacritty_terminal 0.26.0

Vendored from crates.io `alacritty_terminal` 0.26.0 (Apache-2.0,
`LICENSE-APACHE` as published) and published as `zgx-alacritty-terminal`, its
library still `alacritty_terminal`, so zgx's crates build from crates.io
without a `[patch]` section. The changes are marked `zgx:` in the source. The
crate's reference tests (`tests/`) are left out.

## Why

`index.rs` compared `usize` with a `Column` and `i32` with a `Line` from the
primitive's side (`impl PartialOrd<Column> for usize`, and `PartialEq`). Such
an impl is seen by every crate whose dependency graph holds this one: there
`usize` compares with two types, and a comparison whose type was inferred
from `usize` having one no longer compiles. An app turning on zgx-desktop's
`terminal` feature found code of its own broken (`range.contains(i)`, with
`i: &&usize`, in zgx's comparisons demo).

## What

The macro making the index types no longer implements `PartialEq` and
`PartialOrd` on the primitive for the index type. A `Column` or `Line` still
compares with its primitive (`column < 5`); the primitive on the left takes
the index type's (`Line(1) == line`). Three assertions in the grid's tests
compared that way and now name the index type.

## Checked

The crate's 132 unit tests pass. zgx-terminal's
`usize_compares_only_with_itself` compiles only while no crate in its graph
implements a comparison of `usize` with another type.

## Ptys opened one at a time (macOS)

macOS's `openpty` isn't safe to call from two threads at once while ptys
are in use: 16 threads opening, each passing a byte through its pty and
closing it, failed about once in 4,000 calls with errno -6, and an
engine-drawn terminal didn't start ("couldn't start: Unknown error: -6");
one thread never failed in 7,200. `tty::open_pty` opens the pair behind a
process-wide lock, and `tty::new` uses it. zgx-terminal's
`ptys_opened_at_once_from_many_threads_all_open` failed 9 runs in 10
without the lock, none with it. Version 0.26.1.

## A program's last output, read after it exits (Windows)

ConPTY renders what a program last wrote after the program has gone, as
the pseudoconsole closes, and then ends the output pipe. zgx-terminal read
the pipe once at the child's exit and dropped the pty, so a program's last
lines were often still to come and never showed: on WING-PC, ping's
statistics after Ctrl+C were missing in 98 of 100 runs once the test sent
Ctrl+C after ping's first reply. `Pty::close_console` closes the
pseudoconsole on a thread of its own (`ClosePseudoConsole` blocks until the
pipe is drained, so it can't run on the thread draining it; `Drop` then
closes nothing), and `Pty::output_finished` says when the pipe has ended
and everything in it was read (`UnblockedReader::finished`: the reader
returns 0 alike for nothing yet and for the end). zgx-terminal's session
reads until then, five seconds at most. Also in 0.26.1.

The pipe's end came only once nothing else held its writing end. `new`
passed the pseudoconsole its ends of the pipes with `into_raw_handle`,
leaving this process's copies open for good, so the output pipe never
ended and every drain waited its five seconds (on WING-PC, 5 of 50
immediate exits under load timed out on the exit). They are now closed once
`CreatePseudoConsole` holds its own (Microsoft's ConPTY guidance), and the
child's thread handle once `CreateProcessW` returns.

## An emoji asked for with VS16 takes two cells

A character shown as text by default that has an emoji presentation (❤,
☺, ✔, ⚠) took one cell with VS16 (U+FE0F) after it, as the selector, a
zero-width character, joined it. Ghostty, kitty and iTerm2 draw it as an
emoji two cells wide (as do Windows Terminal and foot; WezTerm, Alacritty
and VTE keep one), and so do the programs that measure text with Unicode's
widths rather than `wcwidth`: Ink's string-width and ratatui's
unicode-width count `❤️` as 2, so coding agents' lines with ✔️ or ⚠️ were
out of step with what showed. `Term::input` hands VS16 to
`emoji_presentation` (cold, out of line, reached only from the zero-width
branch): when the pair is two wide (`unicode-width`'s `str` width) and the
cell is the character just written, it becomes a wide character with its
spacer, and the cursor goes past both (in insert mode the row after it
shifts a cell first, as `input` shifts it); on the last column it starts
the next line as a wide character does. Programs counting with `wcwidth`
(readline) see one column, as in those terminals, so an edit across such an
emoji puts their cursor a cell off. Mode 2027 (grapheme clustering) isn't
recognized. zgx-terminal's `an_emoji_asked_for_with_vs16_takes_two_cells`
checks the cells, the line's end and the cases left alone. Version 0.26.2.
