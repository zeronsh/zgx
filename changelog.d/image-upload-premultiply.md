---
section: performance
---
- **An image uploads in fewer instructions, a video frame's most of all** (zgx-gpu): premultiplying a straight image's pixels goes sixteen at a time, leaving a block all opaque (a photo's, a video's) as it is and making one all transparent (around a canvas's marks) zeros, and computes the rest exactly with a multiply and shifts in 16 bits, vectorized, where it divided each channel a pixel at a time; red and blue swap a pixel a word as the texels go to Metal. The pixels are the same (every colour and alpha checked). A 1080p clip playing in an app's window went from 2.67 G to 2.04 G instructions a second.
