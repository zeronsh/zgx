---
section: fixed
---
- **Text measured without a renderer no longer keeps every word it shaped** (zgx-gpu, zgx-cosmic-text 0.19.8). A renderer trims cosmic-text's word cache as it draws. Headless hosts, tests and servers measure through `TextServices`, and nothing trimmed theirs, so it grew without bound (about 2.7 KB a frame for 30 churning labels). The text cache now sweeps the word cache as it grows, letting go of words unused lately, so it levels off (30 labels churned through 450,000 values: about 49,000 words, where untrimmed it reaches 105,000 and climbs). `TextServices::release_unused` empties prepared text and words at once, for a host returning to its baseline after a batch. `TextServices::text_cache` returns its cache, as a renderer's does.
