---
section: performance
---
- **A video player no one sees does no work** (zgx-video): `Player::watch(seen)` tells a player whether what it shows is seen; unseen, its thread draws nothing and doesn't wake each frame, its clock runs on, and seen again it draws the frame due then (a looping one as far into its pass as time says). `Player::view` says so itself, from whether any of its box shows and its window shows (not hidden, minimized or covered), and unseen when it goes; a script's player takes `"<name>.watch"` (from `on_visible`, say). A 1080p clip playing hidden in an app went from 1.3–1.6 G instructions a second to none. `NodeHandle::observe_shown` (zgx) says whether a node's window shows.
