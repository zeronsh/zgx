---
section: fixed
---
- **A picture in Markdown that isn't alone in its paragraph is still a picture** (zgx-script): `ui.markdown` made a picture a block only when nothing came before it in its paragraph. Words before it ("Here it is:" then the picture), or another picture on the line before, turned it into its alt text and lost its source. A picture on a line of its own now splits its paragraph there into the words before, the `img` block and the words after, and leaves no empty paragraph behind. A picture among words is a run with `img`, its source, beside its alt text.
