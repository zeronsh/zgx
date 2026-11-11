---
section: changed
---
- **A backdrop blur's three stages run in one compute pass on Metal drawn directly** (zgx-gpu, macOS): averaging down, blurring across and blurring down are dispatches of one compute encoder over every slot of a batch, not three render passes, so a frosted frame sets up two encoders fewer; its pixels are the same bytes and its GPU time falls. Through wgpu (Windows, Linux) they stay render passes, unchanged.
