---
section: fixed
---
- **An app's shader can't take its process down on Windows** (zgx-gpu): FXC, which compiles shaders for Direct3D 12 when the app ships no DXC, has overflowed every stack on some shaders, ending the process from any thread. zgx's own shaders are compiled under a gate; a shader fill or layer shader an app writes is now compiled by FXC in a process of its own (the app's executable, started again as a compiler) and handed to Direct3D 12 as bytecode, kept on disk so it compiles so once a machine. One FXC can't survive draws nothing, and the app goes on. Where the executable can't be started again, isn't zgx's (zgx in a DLL), or wgpu offers no passthrough shaders, it compiles in the process as before; `ZGX_FXC_IN_CHILD=0` keeps it there.
