---
section: performance
---
- **A 3D world's multisampled colour and depth take no memory on Apple's GPUs where no later pass reads them** (zgx-3d): they live within the pass that draws them (`TextureUsages::TRANSIENT`, memoryless on Metal), as the frame's compiled pass list says, so the post chain's resolve and temporal antialiasing keep what they read. At 1920 by 1080 with four samples and the chain off, an M5 rests 58 to 77 MB lower and draws 71 to 98 MB lower; backends that gain nothing by it (Direct3D 12, GL, most desktop Vulkan) are unchanged. `World::memory` counts such targets as none where the device keeps them so.
