---
section: fixed
---
- **Text services let go of the table their shaped words filled** (zgx-gpu): `TextServices::release_unused` emptied cosmic-text's word cache but kept its table at the size it peaked at, 13 MB after shaping a long transcript, so a headless host never got it back. A memory report's "shaped runs" now counts that table with the words.
