---
section: changed
---
- **A build embeds the Luau definitions and lint rules of its features** (zgx-script): the `audio` global's types, `sound =` on controls, `ui.sound3d`, `ui.reverb_zone` and the `audio-…` and `sound3d-no-max` lint rules are in a build with `audio`, `audio-effects`, `audio-streams` or `audio-spatial` and in no other, so an app without sound carries none of them. `types/zgx.d.luau` stays whole, its parts marked by feature; `zgx check` and `zgx lint` check against the features the `zgx` binary was built with, so a project using audio needs a CLI built with them.
