---
section: added
---
- **The engine-drawn terminal shows pictures by the kitty graphics protocol** (zgx-terminal), as `kitten icat`, yazi, chafa and viu send them. Pictures come in the escape codes, whole or chunked, as RGB, RGBA or PNG, zlib-compressed or not. They are placed at the cursor over the cells they need, or stretched over the columns and rows asked for, under the text or over it. They scroll with their lines into the scrollback and go with it, on a clear or by `a=d`, with a 320 MB quota. Reading files or shared memory a program names is refused. A terminal that never shows a picture holds no room for one.
