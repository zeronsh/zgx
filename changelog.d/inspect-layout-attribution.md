---
section: added
---
- **`zgx inspect frames` says where layout's time went** (zgx, zgx-script): each traced frame counts the nodes layout measured afresh (not taken from a measurement kept from before) and their time, and names the costliest by their own time, their children's apart, as the script made them (`layout: 14 measured afresh, 5120 µs; costliest: @88 #chat (column screens/chat.luau:41) …`; `layout` in its JSON). An interaction's `summary` adds them up by node. While nothing traces, a fresh measure pays one relaxed load for it.
