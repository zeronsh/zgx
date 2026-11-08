---
section: changed
---
- **Cancelling a promise or task drops the host work it waits on.** `p:cancel()`, or cancelling a task awaiting it, now drops a file dialog's request, a file read or a `ui.sleep` where it waits, as it already stopped a fetch: Linux's portal closes its picker; a macOS sheet or Windows dialog stays up until answered, its answer unheard. Before, the work ran on and only its answer was dropped. A host action still runs to the end.
