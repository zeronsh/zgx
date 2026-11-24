---
section: changed
---
- **`ClipboardContent` has `types`, and copies hand it whole** (zgx): a literal adds `types: Vec::new()`. It is what `View::on_copy` and `View::on_selection_copy` return. `Ui::copy_from` and `Ui::copy_selection_from` call their `write` with the `ClipboardContent` rather than `(text, html)`: `|content| content.write(platform)` puts it on a `Platform`'s clipboard as the window's copy does.
