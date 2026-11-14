---
section: added
---
- **Text raised off the baseline** (internal for now, subject to change): a superscript span (`ui.sup { "2" }`, `TextSpan::superscript`) or an editor's styled span (`highlight`'s `baseline_shift`, `scale`, `superscript`) draws raised as CSS's `vertical-align` does: a footnote's mark as a true superscript, three quarters of the size and raised a third of it. Its line grows only where the raised text reaches past it; clicks land on it, selection follows it, and a caret in it is raised and sized to it. Text that isn't raised is laid out as before.
