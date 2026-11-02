---
section: added
---
- **Text streams into an editor where it ends** (zgx, zgx-script):
  - `TextEditor::apply_external(range, text)` and `append_external(text)` take an edit made outside the editor (a model's reply, a collaborator) as `set_text` takes the document it makes, same undo and redo, comparing only the edit; the caret at the end follows the stream, and a selection keeps what it selected.
  - An editor bound to a long text follows the edits the text keeps (`zgx::text::Text` remembers its last 16), applying each where it is.
  - A `\r` and a `\n` streamed in different tokens make one line break, not two.
  - Luau's `state:append(text)` (and `append(path, text)`) adds to a long string in place, keeping the edit for a bound editor, and a string growing past 64 KiB becomes one; `zgx lint` flags `state:set(state:get() .. token)` (`append-concat`).
