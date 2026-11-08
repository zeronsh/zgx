---
section: fixed
---
- **What a script's update costs no longer depends on when Luau's collector last ran** (zgx-script): the cache of state converted for scripts held its tables weakly, so a computed's result filed one tick was walked, copied and weighed again the next if a collection had come between, an extra pass that came and went as unrelated code moved the collector's timing. The cache now holds its tables strongly; what no script holds goes in one pass as the window goes idle, after the collection the idle trim already runs. A long list read and let go of still leaves the script heap at idle.
