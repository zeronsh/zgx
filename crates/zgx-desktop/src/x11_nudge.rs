//! winit's X11 backend queues an input method's spot
//! (`set_ime_cursor_area`) and applies it after it next handles an X event.
//! The last preedit of a word is often the last event for a while, so the
//! caret it moves sent its spot nowhere until the next key: IBus left its
//! candidates where a longer preedit had put the caret ("中哦" before
//! "中", one character to the right). A message to the window itself, on
//! winit's own connection, is that next event, at once.
use std::sync::OnceLock;
use winit::{
    raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle},
    window::Window,
};
use x11_dl::xlib;

/// Xlib, and `window`'s display and X window, if it is an X11 window.
fn raw(window: &Window) -> Option<(&'static xlib::Xlib, *mut xlib::Display, xlib::Window)> {
    static XLIB: OnceLock<Option<xlib::Xlib>> = OnceLock::new();
    let x = XLIB.get_or_init(|| xlib::Xlib::open().ok()).as_ref()?;
    let (Ok(display), Ok(handle)) = (window.display_handle(), window.window_handle()) else {
        return None;
    };
    let (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(handle)) =
        (display.as_raw(), handle.as_raw())
    else {
        return None;
    };
    Some((x, display.display?.as_ptr().cast(), handle.window))
}
/// Have winit handle what it queued for `window`'s X11 connection now.
pub(crate) fn nudge(window: &Window) {
    let Some((x, display, xwindow)) = raw(window) else {
        return;
    };
    // SAFETY: winit's display, open while `window` (which holds winit's
    // connection) is borrowed, on the thread that runs its event loop
    // (winit made Xlib thread-safe besides, XInitThreads). The atom is
    // interned on it once: winit's one event loop has one display per
    // process. The event is a zeroed union with its client-message member
    // written, a plain value naming our own window, which winit ignores (an
    // atom it does not know).
    unsafe {
        static ATOM: OnceLock<xlib::Atom> = OnceLock::new();
        let atom = *ATOM.get_or_init(|| (x.XInternAtom)(display, c"ZGX_NUDGE".as_ptr(), 0));
        let mut event: xlib::XEvent = std::mem::zeroed();
        event.client_message = xlib::XClientMessageEvent {
            type_: xlib::ClientMessage,
            serial: 0,
            send_event: 1,
            display,
            window: xwindow,
            message_type: atom,
            format: 32,
            data: xlib::ClientMessageData::new(),
        };
        (x.XSendEvent)(display, xwindow, 0, 0, &mut event);
        (x.XFlush)(display);
    }
}
/// Show `cursor`, another connection's cursor on the same X server (a web
/// page's, GTK's), over `window`. winit doesn't know: the window's own cursor
/// comes back by hiding and showing winit's.
#[cfg(feature = "webkitgtk")]
pub(crate) fn define_cursor(window: &Window, cursor: xlib::Cursor) {
    let Some((x, display, xwindow)) = raw(window) else {
        return;
    };
    // SAFETY: winit's display and window, as `nudge`'s; a cursor is a
    // server resource any client may name, and one freed while a window
    // shows it lives on until the window shows another.
    unsafe {
        (x.XDefineCursor)(display, xwindow, cursor);
        (x.XFlush)(display);
    }
}
