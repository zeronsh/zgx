---
section: fixed
---
- **A mesh set not to cast the sun's shadow no longer shades traced probes** (zgx-3d, `gi` with `lighting`): probes tracing rays shadowed the sun and lamps by every mesh, so a roof set by `World::set_casts_shadows(id, false)` still darkened the floor beneath it as the probes saw it, and the light it bounced. Shadow rays now pass meshes that don't cast; every ray still sees them. Probes drawn as faces already followed the shadow maps.
