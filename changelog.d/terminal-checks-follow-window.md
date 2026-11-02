---
section: fixed
---
- **A terminal moved to another window keeps naming what runs in it** (zgx-terminal): when an engine-drawn terminal's session showed in another window (a tab dragged out, a view mounted elsewhere), its output followed it to that window's tasks, but a check of the process in front already due, and the app resolver's polling, stayed with the first window's. Once that window closed they ended with it, their pending flags left set, and the terminal never named its process or status again. They now move with the session.
