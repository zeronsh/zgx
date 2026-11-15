---
section: performance
---
- **A terminal cell is 16 bytes, not 24** (zgx-terminal): its joined characters, underline colour and hyperlink are kept by its row, so a cell has nothing to drop: clearing a line is a copy of blanks, and text in the default colours is blanked in one copy and given its characters. Printing a coloured 20 MB log takes 25% fewer instructions, and a 10,000-line history 11 MB less memory (35 MB to 24 MB resident). A search through the history reads a candidate's second character before matching the rest: 7% fewer instructions searching 100,000 lines.
