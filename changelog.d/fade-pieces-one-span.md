---
section: fixed
---
- **An inline code chip a fade cut into pieces draws as the chip whole** (zgx, zgx-gpu): a padded span cut into runs (a streamed reply's pieces, each fading in as a run of its own and left apart once faded) was padded at every cut, so a chip with a cut drew 4 points wider for each, its wash notched there, and its line could wrap otherwise than the same text written at once. Runs that go on one padded span (`RichText::continues`: padded alike, in one font, size, background and radius) are now padded at the span's ends alone, their washes and lines drawn as one. Also: an inline code chip's word shaped after the chip was drawn cut at a space (the blank found padded in cosmic-text's shape-run cache) no longer draws 4 points wider.
