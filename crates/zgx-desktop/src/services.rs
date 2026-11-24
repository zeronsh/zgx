//! macOS's Services menu on a window's selection (winit's view answers it,
//! `winit::platform::macos::set_selection_services`): a service takes the
//! selected text, a field's or selectable text's, and one that returns
//! text puts it in place of a field's selection, as in AppKit's text views.
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use winit::window::WindowId;

struct WindowEntry {
    ui: zgx::widgets::WeakUi,
    wake: Rc<dyn Fn()>,
}

thread_local! {
    /// The windows' interfaces, by their winit window.
    static WINDOWS: RefCell<HashMap<WindowId, WindowEntry>> = RefCell::default();
}

/// `window` shows `ui`: the Services menu and macOS `NSTextInputClient`
/// queries act on its focused editor. The first also hands the views their
/// answers and registers the text the app sends and takes.
pub(crate) fn add(window: WindowId, ui: &zgx::widgets::Ui, wake: impl Fn() + 'static) {
    let first = WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        windows.insert(
            window,
            WindowEntry {
                ui: ui.downgrade(),
                wake: Rc::new(wake),
            },
        );
        windows.len() == 1
    });
    if first {
        let services = Rc::new(Windows);
        winit::platform::macos::set_selection_services(Some(services.clone()));
        winit::platform::macos::set_text_input_services(Some(services));
        register_types();
    }
}

/// `window` closed.
pub(crate) fn remove(window: WindowId) {
    WINDOWS.with(|windows| windows.borrow_mut().remove(&window));
}

fn ui(window: WindowId) -> Option<zgx::widgets::Ui> {
    WINDOWS.with(|windows| windows.borrow().get(&window)?.ui.upgrade())
}

fn entry(window: WindowId) -> Option<(zgx::widgets::Ui, Rc<dyn Fn()>)> {
    WINDOWS.with(|windows| {
        let map = windows.borrow();
        let entry = map.get(&window)?;
        Some((entry.ui.upgrade()?, entry.wake.clone()))
    })
}

fn editable_editor(window: WindowId) -> Option<(zgx::widgets::EditorHandle, Rc<dyn Fn()>)> {
    let (ui, wake) = entry(window)?;
    let editor = ui.focused_editor()?;
    if editor.is_read_only() || editor.is_secure() {
        return None;
    }
    Some((editor, wake))
}

struct Windows;
impl winit::platform::macos::SelectionServices for Windows {
    fn selected_text(&self, window: WindowId) -> Option<String> {
        let ui = ui(window)?;
        // A password's `copy` is empty: its text never leaves it.
        let text = match ui.focused_editor() {
            Some(editor) => editor.copy(),
            None if ui.has_text_selection() => ui.selected_text(),
            None => String::new(),
        };
        (!text.is_empty()).then_some(text)
    }
    fn selection_editable(&self, window: WindowId) -> bool {
        ui(window)
            .and_then(|ui| ui.focused_editor())
            .is_some_and(|editor| !editor.is_read_only() && !editor.copy().is_empty())
    }
    fn replace_selection(&self, window: WindowId, text: &str) -> bool {
        let Some(editor) = ui(window).and_then(|ui| ui.focused_editor()) else {
            return false;
        };
        if editor.is_read_only() || editor.copy().is_empty() {
            return false;
        }
        editor.paste(text);
        if let Some((_, wake)) = entry(window) {
            wake();
        }
        true
    }
}

impl winit::platform::macos::TextInputServices for Windows {
    fn selected_range(&self, window: WindowId) -> Option<(usize, usize)> {
        let (editor, _) = editable_editor(window)?;
        let mut e = editor.editor.borrow_mut();
        if let Some(preedit) = e.preedit() {
            let (a, b) = preedit
                .cursor
                .unwrap_or((preedit.text.len(), preedit.text.len()));
            let to_u16 = |at: usize| {
                preedit.text[..at.min(preedit.text.len())]
                    .encode_utf16()
                    .count()
            };
            let (ua, ub) = (to_u16(a), to_u16(b));
            let start = e.selection().range().start;
            let before = e.byte_to_utf16(start);
            return Some((before + ua, ub.saturating_sub(ua)));
        }
        let range = e.selection().range();
        let start = e.byte_to_utf16(range.start);
        let end = e.byte_to_utf16(range.end);
        Some((start, end.saturating_sub(start)))
    }

    fn marked_range(&self, window: WindowId) -> Option<(usize, usize)> {
        let (editor, _) = editable_editor(window)?;
        let mut e = editor.editor.borrow_mut();
        let len = e.preedit()?.text.encode_utf16().count();
        let start = e.selection().range().start;
        let before = e.byte_to_utf16(start);
        Some((before, len))
    }

    fn substring(
        &self,
        window: WindowId,
        location: usize,
        length: usize,
    ) -> Option<(usize, String)> {
        let (editor, _) = editable_editor(window)?;
        let mut e = editor.editor.borrow_mut();
        let preedit = e.preedit().map(|p| p.text.clone());
        let committed_len = e.utf16_len();
        let (before, cut, preedit_u16) = match &preedit {
            Some(text) => {
                let sel = e.selection().range();
                let before = e.byte_to_utf16(sel.start);
                let cut = e.byte_to_utf16(sel.end) - before;
                (before, cut, text.encode_utf16().count())
            }
            None => (0, 0, 0),
        };
        let doc_len = committed_len - cut + preedit_u16;
        let start = location.min(doc_len);
        let end = location.saturating_add(length).min(doc_len);
        if start >= end {
            return Some((start, String::new()));
        }

        // Convert UTF-16 range in the committed text, snapping to full Unicode
        // scalar values so we never split a surrogate pair.
        let committed_slice = |e: &mut zgx::text_edit::TextEditor, from: usize, to: usize| -> (usize, String) {
            if from >= to {
                return (from, String::new());
            }
            let a = e.utf16_to_byte(from);
            let actual_from = e.byte_to_utf16(a);
            let mut b = e.utf16_to_byte(to);
            if e.byte_to_utf16(b) < to {
                b += e.slice(b..e.len()).chars().next().map_or(0, char::len_utf8);
            }
            (actual_from, e.slice(a..b).into_owned())
        };

        let Some(preedit_text) = preedit else {
            return Some(committed_slice(&mut e, start, end));
        };

        let (p0, p1) = (before, before + preedit_u16);
        let shift = |at: usize| at - preedit_u16 + cut;
        let mut out = String::new();
        let mut actual_start = start;
        let mut set_start = false;
        if start < p0 {
            let (s, chunk) = committed_slice(&mut e, start, end.min(p0));
            actual_start = s;
            set_start = true;
            out.push_str(&chunk);
        }
        if end > p0 && start < p1 {
            let rel_from = start.saturating_sub(p0);
            let rel_to = end.min(p1) - p0;
            let mut u16_pos = 0usize;
            let mut byte_from = None;
            let mut byte_to = preedit_text.len();
            let mut snapped_from = rel_from;
            for (idx, ch) in preedit_text.char_indices() {
                let next_pos = u16_pos + ch.len_utf16();
                if byte_from.is_none() && next_pos > rel_from {
                    byte_from = Some(idx);
                    snapped_from = u16_pos;
                }
                if next_pos >= rel_to {
                    byte_to = idx + ch.len_utf8();
                    break;
                }
                u16_pos = next_pos;
            }
            if !set_start {
                actual_start = p0 + snapped_from;
                set_start = true;
            }
            if let Some(bf) = byte_from {
                out.push_str(&preedit_text[bf..byte_to]);
            }
        }
        if end > p1 {
            let (s, chunk) = committed_slice(&mut e, shift(start.max(p1)), shift(end));
            if !set_start {
                actual_start = s + preedit_u16 - cut;
            }
            out.push_str(&chunk);
        }
        Some((actual_start, out))
    }

    fn replace_range(
        &self,
        window: WindowId,
        location: usize,
        length: usize,
        text: &str,
    ) -> bool {
        let Some((editor, wake)) = editable_editor(window) else {
            return false;
        };
        let (from, to) = {
            let mut e = editor.editor.borrow_mut();
            let committed_len = e.utf16_len();
            let (start, end) = if let Some(preedit_u16) =
                e.preedit().map(|p| p.text.encode_utf16().count())
            {
                let sel = e.selection().range();
                let before = e.byte_to_utf16(sel.start);
                let cut = e.byte_to_utf16(sel.end) - before;
                let doc_len = committed_len - cut + preedit_u16;
                let p0 = before;
                let p1 = before + preedit_u16;
                let map_doc = |at: usize, is_end: bool| {
                    let at = at.min(doc_len);
                    if at < p0 {
                        at
                    } else if at <= p1 {
                        if is_end { before + cut } else { before }
                    } else {
                        at - preedit_u16 + cut
                    }
                };
                let s = map_doc(location, false);
                let t = map_doc(location.saturating_add(length), true).max(s);
                (s.min(committed_len), t.min(committed_len))
            } else {
                let s = location.min(committed_len);
                let t = location.saturating_add(length).min(committed_len);
                (s, t)
            };
            let a = e.utf16_to_byte(start);
            let mut b = e.utf16_to_byte(end);
            if e.byte_to_utf16(b) < end {
                b += e.slice(b..e.len()).chars().next().map_or(0, char::len_utf8);
            }
            (a, b)
        };
        let changed = editor.replace_text(from..to, text);
        if changed || from != to || !text.is_empty() {
            wake();
        }
        true
    }
}

/// The app sends and takes plain text through Services.
fn register_types() {
    use objc2_app_kit::{NSApplication, NSPasteboardTypeString};
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    // SAFETY: AppKit's pasteboard type constant, on the main thread.
    let types = objc2_foundation::NSArray::from_slice(&[unsafe { NSPasteboardTypeString }]);
    NSApplication::sharedApplication(mtm).registerServicesMenuSendTypes_returnTypes(&types, &types);
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::platform::macos::SelectionServices;
    use zgx::compose::prelude::*;

    #[test]
    fn services_take_the_selection_and_put_their_text_in_a_fields_place() {
        let mut ui = zgx::widgets::Ui::new(400., 300.);
        let value = ui.signal(String::from("make this plain"));
        let secret = ui.signal(String::from("hunter2"));
        let view = ui.mount(
            column()
                .child(
                    text_input("Name", value.clone())
                        .id("field")
                        .size(200., 30.),
                )
                .child(
                    text_input("Password", secret)
                        .secure(true)
                        .id("secret")
                        .size(200., 30.),
                )
                .child(column().selectable(true).child(text("Display text"))),
        );
        ui.prepare_frame();
        // A placeholder id, never a window's.
        let window = WindowId::dummy();
        WINDOWS.with(|windows| {
            windows.borrow_mut().insert(
                window,
                WindowEntry {
                    ui: ui.downgrade(),
                    wake: Rc::new(|| {}),
                },
            );
        });
        let services = Windows;
        assert_eq!(services.selected_text(window), None, "nothing selected");
        // A field's selection: sent, and replaced by what returns.
        let field = view.find("field").unwrap();
        ui.input.focus(&ui.scene, Some(field));
        ui.editor(field).unwrap().set_selection(5, 9);
        assert_eq!(services.selected_text(window).as_deref(), Some("this"));
        assert!(services.selection_editable(window));
        assert!(services.replace_selection(window, "it"));
        assert_eq!(value.get(), "make it plain");
        // A password's: never sent.
        let secret = view.find("secret").unwrap();
        ui.input.focus(&ui.scene, Some(secret));
        ui.editor(secret).unwrap().set_selection(0, 7);
        assert_eq!(services.selected_text(window), None);
        assert!(!services.selection_editable(window));
        // Selected display text: sent, not replaced.
        ui.input.focus(&ui.scene, None);
        let label = {
            let scene = ui.scene.borrow();
            let mut stack = vec![scene.root()];
            let mut found = None;
            while let Some(node) = stack.pop() {
                if matches!(scene.kind(node), zgx::scene::NodeKind::Text { text, .. } if &**text == "Display text")
                {
                    found = Some(node);
                }
                stack.extend(scene.children(node).iter().copied());
            }
            found.unwrap()
        };
        assert!(ui.select_text(
            zgx::selection::TextPoint {
                node: label,
                offset: 0
            },
            zgx::selection::TextPoint {
                node: label,
                offset: 7
            },
        ));
        assert_eq!(services.selected_text(window).as_deref(), Some("Display"));
        assert!(!services.selection_editable(window));
        assert!(!services.replace_selection(window, "x"));
        remove(window);
        assert_eq!(services.selected_text(window), None, "closed");
    }

    #[test]
    fn text_input_services_report_utf16_context_and_replace_ranges() {
        use winit::platform::macos::TextInputServices;
        let mut ui = zgx::widgets::Ui::new(400., 300.);
        let value = ui.signal(String::from("a😀1."));
        let secret = ui.signal(String::from("hunter2"));
        let ro = ui.signal(String::from("locked"));
        let view = ui.mount(
            column()
                .child(text_input("Name", value.clone()).id("field").size(200., 30.))
                .child(text_input("Password", secret).secure(true).id("secret").size(200., 30.))
                .child(text_input("RO", ro).read_only(true).id("ro").size(200., 30.)),
        );
        ui.prepare_frame();
        let woke = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let woke_cb = woke.clone();
        let window = WindowId::dummy();
        WINDOWS.with(|windows| {
            windows.borrow_mut().insert(
                window,
                WindowEntry {
                    ui: ui.downgrade(),
                    wake: Rc::new(move || woke_cb.set(woke_cb.get() + 1)),
                },
            );
        });
        let services = Windows;
        let field = view.find("field").unwrap();
        ui.input.focus(&ui.scene, Some(field));
        // "a" (1) + "😀" (2) + "1" (1) + "." (1) = 5 UTF-16 units; caret at end.
        assert_eq!(services.selected_range(window), Some((5, 0)));
        assert_eq!(services.marked_range(window), None);
        assert_eq!(services.substring(window, 3, 2), Some((3, "1.".to_owned())));
        assert_eq!(services.substring(window, 1, 2), Some((1, "😀".to_owned())));
        // Mid-surrogate range snaps to full scalar value boundaries.
        assert_eq!(services.substring(window, 2, 1), Some((1, "😀".to_owned())));
        // Replace "." at UTF-16 offset 4 with "。" (double-period in macOS Pinyin).
        assert!(services.replace_range(window, 4, 1, "。"));
        assert_eq!(value.get(), "a😀1。");
        assert_eq!(services.selected_range(window), Some((5, 0)));
        assert!(woke.get() >= 1);
        // Active preedit in the middle of the document reports preedit-inclusive
        // coordinates and never eats trailing committed text on replace_range.
        let editor = ui.editor(field).unwrap();
        editor.set_selection(1, 1); // right after "a", before "😀1。"
        editor.set_composition("ni", Some((2, 2)), &[]);
        assert_eq!(services.marked_range(window), Some((1, 2)));
        assert_eq!(services.selected_range(window), Some((3, 0)));
        assert_eq!(services.substring(window, 0, 5), Some((0, "ani😀".to_owned())));
        assert!(services.replace_range(window, 1, 2, "你"));
        assert_eq!(value.get(), "a你😀1。");
        // Password and read-only fields expose no text input context.
        ui.input.focus(&ui.scene, view.find("secret"));
        assert_eq!(services.selected_range(window), None);
        assert_eq!(services.substring(window, 0, 7), None);
        assert!(!services.replace_range(window, 0, 1, "x"));
        ui.input.focus(&ui.scene, view.find("ro"));
        assert_eq!(services.selected_range(window), None);
        assert_eq!(services.substring(window, 0, 6), None);
        assert!(!services.replace_range(window, 0, 1, "x"));
        remove(window);
    }
}
