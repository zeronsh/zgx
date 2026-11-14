---
section: performance
---
- **Plain text written into a fresh line is stored without a check per cell** (zgx-terminal): a flood writes each line into a row the scroll just reset, whose cells hold nothing, so they're now stored straight instead of each being read for a wide character and an extra first. The parser's share of printing a coloured 20 MB log went from 378M instructions to 243M, 17% of the whole.
