---
section: added
---
- **Sound for Luau apps in the browser** (zgx-dom, `audio`): `zgx_dom::install_audio(&host)` plays an app's `audio` through zgx-audio's mixer, the desktop's, pulled by an audio worklet. A sound's file is fetched through the host's transport and decoded in Rust. Nothing is made before the first sound, and the context is suspended once the quiet hold has passed. A strict Content-Security-Policy needs `script-src blob:`, or the worklet served and named with `install_audio_at`.
