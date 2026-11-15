//! A text area's atoms under the pointer (`View::on_atom_press`,
//! `View::on_atom_hover`): a chip pressed in place, and the one the pointer
//! is over with its box, found where the editor keeps its atoms. Made only
//! for a text area that asks: one that doesn't has no listener and no cost.
use crate::{
    input::{EventContext, EventPhase, InputEvent, PointerButton},
    scene::Rect,
    text_edit::Atom,
    widgets::EditorHandle,
};
use std::cell::{Cell, OnceCell, RefCell};

/// How far a press may move and still be a press in place, in points: past
/// it, the press selects text as a drag does.
const SLOP: f32 = 4.;

/// An atom pressed and let go in place (`View::on_atom_press`).
pub struct AtomPress<'a> {
    /// The atom, as `EditorHandle::atoms` gives it.
    pub atom: &'a Atom,
    /// Where it was let go across the window, in points.
    pub x: f32,
    /// Where it was let go down the window, in points.
    pub y: f32,
    /// The text its range is in.
    pub text: &'a str,
}

/// The atom the pointer is over, or none as it leaves one
/// (`View::on_atom_hover`).
pub struct AtomHover<'a> {
    /// The atom, as `EditorHandle::atoms` gives it.
    pub atom: Option<&'a Atom>,
    /// Its box on the line under the pointer, in the window's points.
    pub area: Option<Rect>,
    /// The text its range is in.
    pub text: &'a str,
}

type PressListener = Box<dyn FnMut(&AtomPress<'_>) -> bool>;
type HoverListener = Box<dyn FnMut(&AtomHover<'_>)>;

/// A primary press that began on an atom.
#[derive(Clone, Copy)]
struct Pressed {
    x: f32,
    y: f32,
    atom: u64,
    /// The selection before the press, put back if the press is taken.
    before: (usize, usize),
}

/// What a text area's atom listeners keep.
#[derive(Default)]
pub(crate) struct AtomPointer {
    editor: OnceCell<EditorHandle>,
    press: RefCell<Option<PressListener>>,
    hover: RefCell<Option<HoverListener>>,
    pressed: Cell<Option<Pressed>>,
    /// A primary button held over it (no hover is told while it drags).
    down: Cell<bool>,
    /// A taken press's selection, kept through the editor's own handling of
    /// that release until its next input.
    keep: Cell<Option<(usize, usize)>>,
    /// The pointer's last place over it.
    pointer: Cell<Option<(f32, f32)>>,
    /// The atom told hovered, and its box.
    hovered: Cell<Option<(u64, Rect)>>,
}

impl AtomPointer {
    pub(crate) fn set_press(&self, told: impl FnMut(&AtomPress<'_>) -> bool + 'static) {
        *self.press.borrow_mut() = Some(Box::new(told));
    }
    pub(crate) fn set_hover(&self, told: impl FnMut(&AtomHover<'_>) + 'static) {
        *self.hover.borrow_mut() = Some(Box::new(told));
    }
    /// Its text area, mounted: what moves its atoms (a scroll, a new
    /// layout, an edit) is followed after each draw.
    pub(crate) fn attach(self: &std::rc::Rc<Self>, editor: EditorHandle) {
        let pointer = std::rc::Rc::downgrade(self);
        editor.on_drawn(move || {
            if let Some(pointer) = pointer.upgrade() {
                pointer.drawn();
            }
        });
        let _ = self.editor.set(editor);
    }
    /// An input at the text area, before its own handling of it.
    pub(crate) fn event(&self, cx: &mut EventContext) {
        if cx.phase != EventPhase::Target {
            return;
        }
        let Some(editor) = self.editor.get() else {
            return;
        };
        if !matches!(cx.event, InputEvent::PointerUp { .. }) {
            self.keep.set(None);
        }
        match cx.event {
            InputEvent::PointerDown {
                x,
                y,
                button: PointerButton::Primary,
            } => {
                self.down.set(true);
                let pressed = (self.press.borrow().is_some())
                    .then(|| editor.atom_at(x, y))
                    .flatten()
                    .map(|(atom, _)| Pressed {
                        x,
                        y,
                        atom: atom.id,
                        before: editor.selection(),
                    });
                self.pressed.set(pressed);
            }
            InputEvent::PointerMove { x, y } => {
                if let Some(pressed) = self.pressed.get()
                    && (x - pressed.x).hypot(y - pressed.y) > SLOP
                {
                    self.pressed.set(None);
                }
                self.pointer.set(Some((x, y)));
                if !self.down.get() {
                    self.hover_at(editor, Some((x, y)));
                }
            }
            InputEvent::PointerUp {
                x,
                y,
                button: PointerButton::Primary,
            } => {
                self.down.set(false);
                if let Some(pressed) = self.pressed.take()
                    && (x - pressed.x).hypot(y - pressed.y) <= SLOP
                    && self.pressed_atom(editor, pressed, x, y)
                {
                    // Taken: the caret stays where it was, not in the chip.
                    self.keep.set(Some(pressed.before));
                    editor.set_selection(pressed.before.0, pressed.before.1);
                }
                self.pointer.set(Some((x, y)));
                self.hover_at(editor, Some((x, y)));
            }
            InputEvent::PointerCancel => {
                self.down.set(false);
                self.pressed.set(None);
            }
            InputEvent::PointerLeave => {
                self.pointer.set(None);
                self.hover_at(editor, None);
            }
            _ => {}
        }
    }
    /// Tell the press listener of `pressed`'s atom, if it is still there;
    /// whether it took the press.
    fn pressed_atom(&self, editor: &EditorHandle, pressed: Pressed, x: f32, y: f32) -> bool {
        let Some(atom) = editor.atoms().into_iter().find(|a| a.id == pressed.atom) else {
            return false;
        };
        // Taken while it runs: one it causes is not told again.
        let Some(mut told) = self.press.borrow_mut().take() else {
            return false;
        };
        let text = editor.editor.borrow().text().into_owned();
        let taken = told(&AtomPress {
            atom: &atom,
            x,
            y,
            text: &text,
        });
        let mut slot = self.press.borrow_mut();
        if slot.is_none() {
            *slot = Some(told);
        }
        taken
    }
    /// The atom under `at` (none off it), told if it is not the one told.
    fn hover_at(&self, editor: &EditorHandle, at: Option<(f32, f32)>) {
        if self.hover.borrow().is_none() {
            return;
        }
        let hit = at.and_then(|(x, y)| editor.atom_at(x, y));
        let now = hit.as_ref().map(|(atom, area)| (atom.id, *area));
        if now == self.hovered.get() {
            return;
        }
        self.hovered.set(now);
        let Some(mut told) = self.hover.borrow_mut().take() else {
            return;
        };
        let text = editor.editor.borrow().text().into_owned();
        told(&AtomHover {
            atom: hit.as_ref().map(|(atom, _)| atom),
            area: now.map(|(_, area)| area),
            text: &text,
        });
        let mut slot = self.hover.borrow_mut();
        if slot.is_none() {
            *slot = Some(told);
        }
    }
    /// After a draw: a taken press's selection kept, and the hovered atom
    /// followed where a scroll or a new layout moved it.
    fn drawn(&self) {
        let Some(editor) = self.editor.get() else {
            return;
        };
        if let Some((anchor, focus)) = self.keep.get()
            && editor.selection() != (anchor, focus)
        {
            editor.set_selection(anchor, focus);
        }
        if self.hovered.get().is_some() && !self.down.get() {
            self.hover_at(editor, self.pointer.get());
        }
    }
}
