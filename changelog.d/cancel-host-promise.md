---
section: changed
---
- **Cancelling a promise or task drops the host work it waits on.** `p:cancel()`, or cancelling a task awaiting it, now drops a file dialog's request or a `ui.sleep` where it waits and stops waiting on a file read (its thread finishes on its own), as it already stopped a fetch: Linux's portal closes its picker; a macOS sheet or Windows dialog stays up until answered, its answer unheard. Before, the script went on waiting for the host and only dropped the answer. A host action still runs to the end.
