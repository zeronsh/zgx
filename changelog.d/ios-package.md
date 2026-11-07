---
section: added
---
- **An iOS app packaged for the App Store, unsigned** (iOS): `scripts/ios/package.py` builds a manifest's app into an `.xcarchive` and an unsigned `.ipa`, with its icons compiled from one image, an Info.plist from a template (background audio, the microphone's reason, link schemes) and an entitlements file for signing. The iOS host sets the app's audio session category from its Info.plist (`ZGXAudioSessionCategory`) at launch.
