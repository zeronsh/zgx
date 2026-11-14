//! The browser as zgx's `Platform` (`zgx::platform`): the clipboard, text
//! and HTML both ways, and links opened in a tab of their own. Provided
//! around every mounted view (`mount`), as a desktop window provides its
//! own, so `cx.service::<PlatformService>()` and Luau's `ui.clipboard`
//! reach it.
//!
//! A page reads the clipboard only inside a paste event, so reads give the
//! paste being handled, else what the page last copied, cut, pasted or
//! wrote. A key a keymap binds to paste (⌘V, Ctrl+V) runs its action when
//! the paste event comes, so it reads what is pasted (`js/zgx-dom.js`).
//! Writes go through `navigator.clipboard.write`, `text/plain` with
//! `text/html` beside it.

use crate::js;
use std::path::PathBuf;
use zgx::platform::{FilePicker, Pending, Platform};

pub(crate) struct Browser;

impl Platform for Browser {
    fn clipboard_text(&self) -> Option<String> {
        js::zgx_clipboard_text()
    }
    fn set_clipboard_text(&self, text: String) -> bool {
        js::zgx_clipboard_write(&text, None)
    }
    fn clipboard_html(&self) -> Option<String> {
        js::zgx_clipboard_html()
    }
    fn set_clipboard_rich_text(&self, text: String, html: String) -> bool {
        js::zgx_clipboard_write(&text, Some(html))
    }
    fn open_url(&self, url: &str) -> bool {
        js::zgx_open_url(url)
    }
    fn open_files(&self, picker: FilePicker) -> Pending<Result<Vec<PathBuf>, String>> {
        let _ = picker;
        Box::pin(async { Err("no file dialog in the browser yet".into()) })
    }
    fn save_file(&self, picker: FilePicker) -> Pending<Result<Option<PathBuf>, String>> {
        let _ = picker;
        Box::pin(async { Err("no file dialog in the browser yet".into()) })
    }
}
