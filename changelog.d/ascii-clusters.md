---
section: performance
---
- **Text measured without a shaper reads an ASCII text's graphemes from its bytes** (zgx): each byte is a grapheme but a carriage return before a line feed, so `FallbackTextLayout` no longer runs Unicode's segmentation over ASCII text. A headless host (tests, offscreen hosts) measures text 6% faster in whole runs: script-update −6.6%, mount −6.1%, with the same sizes and carets.
