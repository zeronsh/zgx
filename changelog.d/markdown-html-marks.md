---
section: added
---
- **Markdown shows inline HTML's marks, when asked** (zgx-script): `ui.markdown(text, { html_marks = true })`, `ui.markdown_view { text, html_marks = true }` and `markdown::Options::html_marks` style `b`/`strong`, `i`/`em`, `s`/`del`/`strike` and `code`/`kbd`/`samp`/`tt`, link an `a href` (relative, or a protocol GitHub's schema keeps), and break the line at `br`, as a web chat rendering markdown with raw HTML and GitHub's sanitizing schema does. Other tags are dropped with their text kept, block HTML stays dropped, and without the option nothing changes.
- **Markdown reads its options from the table given** (zgx-script): `ui.markdown`'s and `ui.markdown_view`'s `breaks`, `paths`, `html_marks` and `copy` are read raw, by names a VM makes once, so a call costs no string made or protected call for each; options a props table only inherits through a metatable's `__index` are no longer read.
