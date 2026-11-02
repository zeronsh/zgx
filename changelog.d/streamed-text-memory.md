---
section: fixed
---
- **A long text streamed into gives back its edits once the stream stops** (zgx, zgx-script): the last edits a long text keeps for its readers (an editor or `ui.markdown_view` following a reply by its appends) stayed for as long as it did; they now go as the host goes idle (`Text::forget_edits`), so a reply streamed in holds what the same text set whole does. Appending to a short string copies it once rather than twice, so `state:append` and `Shared::append` never cost more than setting the joined text.
