---
section: fixed
---
- **A flex item that names its minimum width is no longer laid out at zero width** (zgx): taffy measured every item at min-content for an automatic minimum and then used the one it named, so a column given `grow`, a basis of 0 and `min_w = 0` (a chat row's text beside an avatar) laid its paragraphs out a line a word, in Chinese or Japanese a line a character, on every layout. On an iOS simulator one such paragraph made an 83 ms frame scrolling a feed; only an automatic minimum measures min-content now, as later taffy releases do.
