---
section: added
---
- **The `zgx_monitor` example follows an app's process tree and shows the names an app gives its processes** (zgx-desktop): `--attach <name>` (or its Attach menu) narrows the sidebar to the processes of that name and every process they started. On macOS an app can name its helpers, which other tools can't rename, and publish counts of its own (its open tabs) in `$TMPDIR/zgx-monitor/labels-<pid>`, written only while the monitor's heartbeat beside it is fresh; the example's module documentation gives the format. Shipped in an app bundle, the monitor runs the scripts in `Contents/Resources/monitor/`.
