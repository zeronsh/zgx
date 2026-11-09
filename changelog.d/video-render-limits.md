---
section: fixed
---
- **A video's scripts run under a render's limits** (`zgx-video`): a composition that mounts a whole app (its first build) no longer stops with "script exceeded its time budget"; a render's calls may take a minute and its load two, since nothing waits on a frame (`zgx_video::script::RENDER_LIMITS`).
