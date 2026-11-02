---
section: fixed
---
- **A clickable chip span is one pill, not underlined unless asked** (zgx, zgx-script):
  - **Underline:** a span with `on_click` was underlined even with `underline = false`, since false was taken as not given. `underline = false` now leaves a link not underlined at rest (`TextSpan::no_underline`), though a hover's underline still shows. It wins over an underline's style, colour or thickness too.
  - **One wash:** a span with an icon and a background was washed in three pieces: the icon's room, its text and the padding after it. Their backgrounds now join into one rounded wash per line. Spans side by side keep their own.
