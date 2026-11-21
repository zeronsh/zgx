---
section: changed
---
- **`ClipboardContent` has `types`, and copies hand it whole** (zgx): `ClipboardContent` (what `View::on_copy` and `View::on_selection_copy` return) has a `types` field: build one with `ClipboardContent::new(text, html)` or add `types: Vec::new()`. `Ui::copy_from` and `Ui::copy_selection_from` call their `write` with the `ClipboardContent` rather than `(text, html)`: `|content| content.write(platform)` puts it on a `Platform`'s clipboard as the window's copy does.
