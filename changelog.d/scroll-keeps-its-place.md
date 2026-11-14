---
section: fixed
---
- **A scroll view goes back to its place once content that was short for a while grows back** (zgx): a view resting at an offset its content can no longer reach (a list built again, a section swapped, a page made before its rows arrive) is clamped as before, and returns to where it rested as the content grows to hold it again, unless the person or a script moved it in between. Before, the clamped value overwrote the offset, so a single short frame left the view near the top for good. An offset set past the end is clamped as before and not kept.
