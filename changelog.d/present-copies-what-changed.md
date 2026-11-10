---
section: performance
---
- **On Linux a present copies into the window only what changed** (zgx-gpu, Vulkan): each frame copied its whole retained image into the swap chain's, a memcpy of the window under a software rasterizer for a hover over two cells. Each swap chain image now takes the changes of the frames it missed (its age, by image), and the compositor is told which regions changed (`VK_KHR_incremental_present`, where the device has it), so it redraws those alone. Anything uncertain (a new or resized swap chain, a suboptimal image, an expose, the window shown again) copies all of it. On sway and X11, frame_clock's moving box copied 11,700 of 124,800 pixels a present.
