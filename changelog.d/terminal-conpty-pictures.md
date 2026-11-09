---
section: fixed
---
- **A terminal on Windows announces pictures only where they can arrive** (zgx-terminal): Windows' own ConPTY drops the kitty graphics and sixel strings, so a program saw sixel announced and sent pictures that never showed. There, device attributes no longer include sixel and picture commands go unanswered, as in a terminal without pictures. Shipped beside the app, Windows Terminal's `conpty.dll` and `OpenConsole.exe` (MIT, `Microsoft.Windows.Console.ConPTY` 1.25.260930003 tested) carry them, and pictures show.
