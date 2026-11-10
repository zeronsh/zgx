---
section: fixed
---
- **Simplified Chinese after Japanese draws in a Chinese face** (zgx-gpu): in a line with a Japanese sentence before a Chinese one, the simplified characters Japanese faces lack (车, 见) drew in Arial Unicode MS on macOS, the first of every installed face the fallback search tried. A Japanese or Korean run now tries the Chinese faces next, as Core Text does. That last search, every face loaded in turn (some 400 on iOS, 14 ms on a paragraph's first layout), now runs once a character: what it found is kept until the fonts change.
