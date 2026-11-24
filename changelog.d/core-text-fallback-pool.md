---
section: performance
---
- **Text laid out without an event loop keeps nothing Core Text autoreleased** (zgx-text, macOS): looking up the face for a character the text's fonts lack hands back objects Core Text autoreleases, which went only when an autorelease pool drained. An app's event loop drains one each turn, but text laid out without one (a headless renderer, a thread of its own) kept them: some 9 MB at rest after a page of new Chinese characters. Each lookup now drains its own pool.
