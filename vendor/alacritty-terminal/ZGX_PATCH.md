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
emoji puts their cursor a cell off (0.26.3 lets them reset grapheme
clustering, below, for one column). zgx-terminal's `an_emoji_asked_for_with_vs16_takes_two_cells`
checks the cells, the line's end and the cases left alone. Version 0.26.2.

## Grapheme clustering, DEC mode 2027

As Ghostty has it (`Terminal.print`, `unicode/grapheme.zig`), on by
default, set and reset with `CSI ? 2027 h` and `l`, answered by DECRQM
(`CSI ? 2027 ; 1 $ y` set, `2` reset) and on again after a full reset. In
the mode a character above U+00FF continuing the grapheme cluster before
the cursor (UAX #29, `unicode-segmentation`) joins that cell: an emoji ZWJ
sequence, a flag or a skin-toned emoji is one character in two cells (they
took six, two and four). The cluster's width is its first character's,
made two by VS16 after a character with both presentations
(emoji-variation-sequences.txt) or by any character that has width inside
a cluster (a spacing mark, a conjunct consonant, the second regional
indicator), and one by VS15; a regional indicator alone takes two. A
variation selector after something without both presentations, or a
zero-width character continuing nothing, is dropped. Reset, each character
takes its own width (zero-width ones joined to the cell before), as
`wcwidth` counts and a line editor such as readline expects: its cursor
then keeps in step across ❤️, where with the mode set it is a cell off.

Ordinary text pays nothing for it: printable ASCII is told before the
width lookup, and anything else above U+00FF reaches `grapheme_input`
(cold, out of line) only if it is zero-width or may continue a cluster
(`grapheme::may_continue`: most text, Latin to Arabic, CJK, kana, Hangul and
fullwidth forms, is out after a compare or two; a table made from
`unicode-segmentation` takes the rest, and
`the_continuing_table_is_what_segmentation_says` checks it against every
character). Hangul syllables continuing a leading jamo (old Hangul written
in parts), four pictographs in the CJK blocks after a ZWJ, and a character
after a Prepend aren't joined. The fork's `grapheme_tests` check the mode
and its answers, ZWJ sequences, flags, skin tones, both selectors,
combining and spacing marks, conjuncts and the last column; zgx-terminal's
`grapheme_clusters_take_one_cell_or_two_until_mode_2027_is_reset` prints
them through a pty, and
`a_readline_edit_across_an_emoji_keeps_its_cursor_with_mode_2027_reset`
edits across ❤️ in bash. A cell's joined characters (`CellExtra`'s
`zerowidth`) are kept inline for up to four (`SmallVec`), so a cluster, or
a selector or ZWJ joined with the mode reset, costs one allocation rather
than two. Version 0.26.3, depending on `unicode-segmentation` and
`smallvec`.

## Pictures placed on the grid

For the kitty graphics protocol, which zgx-terminal reads (vte passes APC
strings over), each screen keeps the pictures placed on it
(`term::graphics`): which picture, under what number, the rectangle of cells
it covers (its top line a grid line, negative in the scrollback), its z and
what part of it shows. `Term::place`, `placements` and `retain_placements`
are the embedder's. The placements move with the lines they sit on: a
scroll from the top carries them into the scrollback and drops them once
their last line leaves it; a scrolling region moves those starting in it
and drops those leaving it; inserted and deleted lines are such scrolls; a
resize moves them as the screen moves over the scrollback (a reflow of
wrapped lines isn't followed). Clearing the screen (ED 2) drops what it
shows, as kitty does, and moves the scrollback's with the lines the clear
pushes into it (`Grid::clear_viewport` says how many); clearing the
scrollback (ED 3) drops what starts there; a shorter scrollback drops what
its lost lines showed. The alternate screen keeps its own, blank each time
it is entered; a full reset drops both. Text written over a picture leaves
it, as kitty does.

A screen with none allocates nothing and pays one comparison where its
lines move (each move is out of line, `#[cold]`). The fork's
`graphics::tests` check each case and that no placements hold no room.
Also in 0.26.3.
