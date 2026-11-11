---
section: fixed
---
- On WARP (Direct3D 12 on the CPU, as in VMs, remote sessions and machines without a Direct3D 12 GPU), renderers on several threads of one process share one device: a device each made WARP 10.0.26100 crash, or draw nothing, when they drew at once.
