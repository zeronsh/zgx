---
section: performance
---
- **A Markdown stream paused at rest gives back its last answer and table** (zgx-script): a stream quiet for 5 s keeps only its parse, so a reply waiting on a tool call still parses just its tail when it goes on. On a long reply (800 sections) read as a table, that returns 2.16 MB of Rust memory and 1.33 MB of Luau heap; its next chunk converts its list once.
