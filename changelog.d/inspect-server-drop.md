---
section: fixed
---
- **A devtools server stops listening and takes its files when it is dropped** (zgx-script): a host closing a window dropped its `inspect::Server`, which removed `<pid>.sock` while its thread went on accepting on the unlinked socket and the app's `<pid>.json` stayed, so `zgx inspect list` named an app no one could reach. Dropped, a server now stops its listener and removes both files, each only while it is still its own (a server made since at the same path, as when a window is made again, keeps its socket and file).
