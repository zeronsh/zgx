---
section: fixed
---
- **Kerning crosses the space between words** (zgx-cosmic-text): text is shaped a word at a time, so the pairs a font kerns with a space (`, `, `r `, ` T`) were lost and each line ran wider along its length (SF at 24 px/600: 403.0 against Chrome's 401.9 on 'Sphinx of black quartz, judge my vow'; 20 px: 341.0 against 340.8). The last character of a word and the space after it, and the space and the next word's first character, now take the font's pair kerning (in the line shaper and in the prepared words the app draws from), cached per font, weight, optical size and pair. The line measures within 0.02 px of Chrome.
