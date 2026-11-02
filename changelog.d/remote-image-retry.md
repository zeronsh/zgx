---
section: fixed
---
- **A remote picture that fails is asked for again by itself** (zgx-script): after a failed fetch (a dropped connection, a busy server's 5xx), a `ui.image` showing it fetches it again once the backoff passes (5 s, doubling up to 5 minutes), for as long as a node shows it. The backoff only held back a fetch something asked for again, and nothing did: the picture stayed blank, or on its `fallback`, for the life of the node.
