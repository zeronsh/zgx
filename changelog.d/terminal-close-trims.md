---
section: fixed
---
- **Closing a terminal gives back its memory in a window showing no frame** (zgx-terminal). The idle trim arms as a frame presents, so a terminal closed in a covered, hidden or minimized window kept its screen and pictures' memory, freed but not returned, until the window next drew: 71 MB against 36 MB after four screens of pictures. Closing now asks for the trim (`zgx::idle::request`).
