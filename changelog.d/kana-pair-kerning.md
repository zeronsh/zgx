---
section: fixed
---
- **Kana that a line break opportunity separates kern as Core Text and Chrome set them** (zgx-cosmic-text): text is shaped a word at a time, and Japanese has a break opportunity between nearly every kana, so a font's GPOS `kern` pairs (Hiragino Sans W3's ォ before ル, イ before ル) were lost: every kana took the full 13 px at 13 px and each line drifted wider along its length (フォルダにファイルを追加: 156 against Chrome's 154.84). The first kana of a pair now takes the kerning the pair's shaping gives it; the line measures 154.83.
