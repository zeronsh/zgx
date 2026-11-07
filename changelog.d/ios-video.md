---
section: added
---
- **Videos play on iOS** (iOS). `zgx_video::Player` plays a composition in an iOS app, its clips (`video.clip`) decoded by AVFoundation where no ffmpeg runs, turned as a phone's recording says, each frame the one on screen at its time. It keeps 64 MB of drawn frames for scrubbing back there (512 MB on the desktop). `ZGX_IOS_ROTATE` has an app turn itself, to check a rotation on a simulator.
