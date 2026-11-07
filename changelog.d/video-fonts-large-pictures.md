---
section: fixed
---
- **A video draws its large pictures, and names its own fonts** (`zgx-video`): a picture larger than 256 by 256 pixels decodes off the thread a frame after it's first drawn, as a window wants, so a render or still drew it as nothing. A video's frames now decode it as they draw it (`zgx::image::set_whole_frames_on_this_thread`). Fonts in a `fonts` folder beside a composition's script are its text's to name by family (`font = "Geist"`), where before only the system's were.
