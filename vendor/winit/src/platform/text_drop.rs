//! zgx patch: text dragged in from another app (a browser's selection, an
//! editor's) drops where the app's windows say, as it does in AppKit's
//! text views and a browser's fields. Windows hear it while it is over
//! them, and whether they would take it there is the platform's answer to
//! the drag (its pointer's badge, and whether a release drops at all).
//! Files keep their own events (`WindowEvent::DroppedFile`).
//!
//! The text is read once, as it drops: X11 hands it over only then.

use std::cell::RefCell;
use std::rc::Rc;

use crate::window::WindowId;

/// Where the app's windows take text dragged in from other apps. Set once by
/// the app ([`set_text_drops`]). Positions are physical pixels from the
/// top left of the window's content, as `WindowEvent::CursorMoved`'s.
pub trait TextDrops {
    /// A drag carrying text is over `window` at `(x, y)`: whether it would
    /// drop there.
    fn over(&self, window: WindowId, x: f64, y: f64) -> bool;
    /// The drag left `window`, or was let go of where nothing took it.
    fn out(&self, window: WindowId);
    /// The drag was let go of over `window` at `(x, y)`, carrying `text`:
    /// whether it was taken.
    fn dropped(&self, window: WindowId, x: f64, y: f64, text: &str) -> bool;
    /// The same, with the drag's text as HTML where the source offered it
    /// too (macOS: `public.html`; X11 and Wayland: `text/html`; Windows:
    /// `CF_HTML`'s fragment). By default, the text alone.
    fn dropped_with_html(
        &self,
        window: WindowId,
        x: f64,
        y: f64,
        text: &str,
        html: Option<&str>,
    ) -> bool {
        let _ = html;
        self.dropped(window, x, y, text)
    }
}

thread_local! {
    static TEXT_DROPS: RefCell<Option<Rc<dyn TextDrops>>> = const { RefCell::new(None) };
}

/// zgx patch: windows take text dragged in from other apps through `drops`
/// (macOS: a pasteboard's string; X11: XDND's UTF-8 text; Windows:
/// `CF_UNICODETEXT`), with its HTML where offered. Main thread only; none
/// refuses such drags.
pub fn set_text_drops(drops: Option<Rc<dyn TextDrops>>) {
    TEXT_DROPS.with(|cell| *cell.borrow_mut() = drops);
}

pub(crate) fn text_drops() -> Option<Rc<dyn TextDrops>> {
    TEXT_DROPS.try_with(|cell| cell.borrow().clone()).ok().flatten()
}
