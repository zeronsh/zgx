---
section: fixed
---
- **A gradient shown again takes the texture it had** (zgx-gpu): a box with a gradient along one axis that went and came back (a navigation stack's page edge on every push, a scroll edge's fade) made its ramp texture and sampler again each time, keeping the old ones until 256 piled up, so a long session's memory crept. A ramp no box shows now stays while its texture does, freed with other unshown images when the window goes idle.
