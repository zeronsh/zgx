# zgx patch to vte 0.15.0

Vendored from crates.io `vte` 0.15.0 (Apache-2.0 OR MIT, `LICENSE-APACHE`
and `LICENSE-MIT` as published) and published as `zgx-vte`, its library
still `vte`, so zgx's crates build from crates.io without a `[patch]`
section. `zgx-alacritty-terminal` parses with it. The changes are marked
`zgx:` in the source.

## Why

Most of what a terminal reads is plain text: a build's log, a file, a
program's output, with an escape here and there for colour. vte finds the
next escape with `memchr` and checks the text up to it is UTF-8, then hands
it over one character at a time: each character decoded, matched against
the controls, and passed through `Perform::print` and `Handler::input`, and
the character before what follows (for REP) stored after each. In zgx's
`terminal` gate cell (a terminal reading 4 KB at a time, a frame after
each), `Performer::print` with alacritty's `Term::input` inlined into it
was 81 M of 730 M instructions (arm64, cachegrind), 11% of the work.

## What

- `src/lib.rs`: `Perform::print_str(&str)`, a run of printable ASCII
  (`' '..='~'`, characters `print` would take), by default `print` for each
  character. `Parser::ground_dispatch` gives each run of printable ASCII in
  the ground state's text to it in one call, and every other character
  (controls, DEL, anything above ASCII) as before, in order.
- `src/ansi.rs`: `Handler::input_str(&str)`, by default `input` for each
  character, and `Performer::print_str` calling it, with the run's last
  character as the one before what follows.

An implementation that overrides neither sees what it saw before.

## Checked

The crate's 54 unit tests pass, and two of its own:
`a_run_of_printable_ascii_is_printed_in_one_call`, and
`runs_print_what_characters_printed_one_at_a_time_do`, which parses 200
streams of text, controls, escapes, UTF-8 of two to four bytes, C1 controls
raw and encoded and invalid bytes, split into chunks of any length, and
finds what a `Perform` taking runs is given, its runs taken apart, equal to
what one taking characters is given.

With `zgx-alacritty-terminal` 0.26.4 writing a run at once
(`Term::input_str`), `terminal` went from 730.4 M to 665.3 M instructions
(−8.9%), and `terminal-pictures` −0.19%.
