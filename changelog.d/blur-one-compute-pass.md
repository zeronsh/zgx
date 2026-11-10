---
section: changed
---
- **A backdrop blur's three stages run in one compute pass** (zgx-gpu): averaging down, blurring across and blurring down are dispatches of one compute encoder over every slot of a batch, not three render passes, on wgpu and on Metal drawn directly. A frosted frame sets up two encoders fewer; its pixels and GPU work are the same.
