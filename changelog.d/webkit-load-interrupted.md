---
section: fixed
---
- **A web view's load that a policy decision or a plug-in took over is no failure** (zgx-desktop, macOS): WebKit's `FrameLoadInterruptedByPolicyChange` (102) and plug-in-handled load (204) are passed over as `NSURLErrorCancelled` is, so a download link or a redirect the app's policy takes no longer reports a load failure.
