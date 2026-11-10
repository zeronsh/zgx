---
section: performance
---
- **A picture in a list's row built ahead starts decoding before it shows** (zgx, zgx-gpu): a kept JPEG or PNG decoded only as it was first drawn, so in a fast scroll through photos each showed its box empty for frames while its turn came. A row built ahead in spare time now asks for its pictures' pixels at the size they'll be drawn (`ImageData::decode_ahead`, by `zgx::image::fitted_size`, which the renderer uses too), the rows the view goes towards first, nearest first. At most `AHEAD_MOST` (4) wait at once, after any a picture drawn waits for; one whose row goes before its turn isn't decoded. A list with no spare time, or one at rest, asks for nothing.
