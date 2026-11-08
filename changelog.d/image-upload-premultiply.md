---
section: performance
---
- **An image uploads in fewer instructions, a video frame's most of all** (zgx-gpu): premultiplying a straight image's pixels leaves an opaque one (a photo's, a video's: nearly all) as it is, and computes the rest exactly with a multiply and shifts where it divided each channel; red and blue swap a pixel a word as the texels go to Metal. The pixels are the same (every colour and alpha checked). A 1080p clip playing in an app's window went from 2.67 G to 2.04 G instructions a second.
