---
section: performance
---
- **A long string in state is read as often as scripts like, at the cost of one copy** (zgx-script): `get` and `peek` of a text past 64 KiB (a document, a draft) hand every read the script string the first one made, until the text changes; each read made it again, copying and hashing all of it. `state:len()` gives a string's length in bytes and `state:sub(i, j)` its bytes as `string.sub` does, read where the text is held, so a binding that needs a draft's line at the caret, or whether it's empty, never copies the rest. Zeron's composer: a keystroke in a draft of 30,000 lines from 343M instructions to 11M, the same as in a draft of 20.
