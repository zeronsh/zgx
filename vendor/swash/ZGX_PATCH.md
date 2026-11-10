# zgx patch to swash 0.2.10

Vendored from crates.io `swash` 0.2.10 (Apache-2.0 OR MIT) and published as
`zgx-swash` 0.2.11, its library still `swash`. `zgx-cosmic-text` renders glyphs
with it. The changes are marked `zgx:` in the source.

## Colour glyph layers composited exactly

`scale::bitmap::blit` composites each layer of a colour glyph (COLR) over the
ones before it. It divided its 0-255 products by 256 (`>> 8`) where they are
over 255, so a layer fully covered in an opaque colour came out 254, and over
nothing 253: no colour glyph had an opaque pixel, every one showed a little of
what is behind it, and its colours were a little darker. Segoe UI Emoji on
Windows is COLR, so every emoji there was. It now divides by 255, rounded.
