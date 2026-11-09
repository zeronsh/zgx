---
section: added
---
- **A span's text can fade in, drawn in Rust** (zgx, zgx-script): `ui.span { …, fade_in = { start = ui.now(), ms = 200, power = 1.6 } }` takes its text from clear to its colour, easing out. The paragraph recolours the run a frame at a time while it fades (`TextSpan::fade_in`, `TextFade`): no script runs for those frames and nothing lays out again, where a colour function reading `ui.now()` re-ran every span's bindings and rebuilt the paragraph's runs on every frame (a streamed reply fading in its new words ran script at the display's rate).
