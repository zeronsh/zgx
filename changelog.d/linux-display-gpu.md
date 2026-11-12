---
section: fixed
---
- **A window on Linux draws on the GPU its display server composites with** (zgx-gpu): the device a Wayland compositor names in its linux-dmabuf feedback, or the one an X server opens over DRI3 (Xwayland, its compositor's), taken as the Vulkan adapter at its PCI address. wgpu's low-power preference took an integrated GPU wherever there was one, so on a desktop whose displays are on a card beside it each frame was drawn on the integrated GPU and handed across: Hyprland on an RTX 3080 showed a window drawn on a Ryzen's integrated Radeon (RADV) black, and under Xwayland its surface failed to configure (a panic). Where the server names no device, or no adapter is at its address, the low-power one is still taken, as on a laptop whose integrated GPU drives its displays.
