---
section: performance
---
- **ffmpeg holds a video's frames no longer, 1.8 GB less for a 1080p film with sound** (zgx-video): the encoder was told to end at the shorter of the picture and the sound (`-shortest`), and to know which, ffmpeg came to hold 10 s of decoded frames in a video longer than that. The mix is now made in whole frames of the composition rounded down, so it never outlasts the picture, and the flag is gone. The showcase's file is byte-identical, and ffmpeg's peak went from 3.0 GB to 1.2 GB. A file written at another rate than its composition's no longer risks losing its last frame to sound rounded short.
