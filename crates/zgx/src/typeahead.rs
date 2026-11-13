//! Typing to jump in a list of named items, as AppKit's menus and tables and
//! a browser's list boxes do: letters typed within 750 ms of each other
//! build a prefix, and the next item whose name starts with it (ignoring
//! case) is the one; the same letter again cycles through those starting
//! with it.
use crate::time::Instant;
use std::time::Duration;

/// How long a prefix waits for its next letter.
const PAUSE: Duration = Duration::from_millis(750);

/// A list's typed prefix.
#[derive(Default)]
pub(crate) struct Typeahead {
    prefix: String,
    last: Option<Instant>,
}
impl Typeahead {
    /// Forget the prefix (the list closed, or went).
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
    /// `text` typed over `len` items, `current` the one focused: the item
    /// to go to, its name `name(index)` (`None`: unnamed). `None` when no
    /// name starts with the prefix, or `text` is no letter (a control
    /// character).
    pub(crate) fn find(
        &mut self,
        text: &str,
        len: usize,
        current: Option<usize>,
        mut name: impl FnMut(usize) -> Option<String>,
    ) -> Option<usize> {
        let text = bounded_prefix(text)?;
        if len == 0 {
            return None;
        }
        let now = Instant::now();
        if self
            .last
            .is_none_or(|last| now.duration_since(last) > PAUSE)
        {
            self.prefix.clear();
        }
        self.last = Some(now);
        if self.prefix.len() + text.len() > 256 {
            self.prefix.clear();
        }
        // Repeating the same prefix cycles matching items, including
        // labels such as "Aaron" that also match a doubled letter.
        if self.prefix != text {
            self.prefix.push_str(&text);
        }
        let mut find = |prefix: &str| {
            (1..=len)
                .map(|step| current.map_or(step - 1, |index| (index + step) % len))
                .find(|index| name(*index).is_some_and(|name| matches_prefix(&name, prefix)))
        };
        let found = find(&self.prefix);
        if found.is_none() {
            self.prefix = text;
            find(&self.prefix)
        } else {
            found
        }
    }
}

/// List box `root`'s options (`Role::ListBoxOption`) take typing to jump
/// to one by its name, and Page Up and Page Down move a page of them, as a
/// browser's list box and AppKit's tables do. Its arrows, Home and End are
/// its focus group's.
pub(crate) fn mount_list_box(ui: &mut crate::widgets::Ui, root: crate::scene::NodeId) {
    use crate::input::{EventPhase, InputEvent, Key};
    let typed = std::rc::Rc::new(std::cell::RefCell::new(Typeahead::default()));
    let weak = ui.downgrade();
    ui.on_event(root, false, move |cx| {
        if cx.phase == EventPhase::Capture || cx.default_prevented() {
            return;
        }
        let InputEvent::KeyDown { key, modifiers, .. } = &cx.event else {
            return;
        };
        if modifiers.control || modifiers.alt || modifiers.meta {
            return;
        }
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let options = options(&ui, root);
        let Some(at) = options.iter().position(|id| *id == cx.target) else {
            return;
        };
        let target = match key {
            Key::Character(text) => {
                let semantics = ui.semantics.borrow();
                typed
                    .borrow_mut()
                    .find(text, options.len(), Some(at), |index| {
                        semantics.get(options[index]).map(|node| node.label.clone())
                    })
                    .map(|index| options[index])
            }
            Key::PageDown | Key::PageUp => {
                page(&ui, root, &options, at, matches!(key, Key::PageDown))
            }
            _ => return,
        };
        if let Some(target) = target.filter(|id| *id != cx.target) {
            ui.input.focus(&ui.scene, Some(target));
        }
        cx.prevent_default();
        cx.stop_propagation();
    });
}

/// List box `root`'s enabled options, in order.
fn options(ui: &crate::widgets::Ui, root: crate::scene::NodeId) -> Vec<crate::scene::NodeId> {
    let scene = ui.scene.borrow();
    let semantics = ui.semantics.borrow();
    let mut options = Vec::new();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        if id != root
            && semantics
                .get(id)
                .is_some_and(|node| node.role == crate::semantics::Role::ListBoxOption)
            && scene.layout_visible(id)
            && ui
                .input
                .options(id)
                .is_some_and(|o| o.focusable && !o.disabled)
        {
            options.push(id);
        }
        pending.extend(scene.children(id).iter().rev().copied());
    }
    options
}

/// The option a page down (`down`) or up from `options[at]`: the furthest
/// one within the list's visible height (its scroll view's, else its
/// own), at least the next.
fn page(
    ui: &crate::widgets::Ui,
    root: crate::scene::NodeId,
    options: &[crate::scene::NodeId],
    at: usize,
    down: bool,
) -> Option<crate::scene::NodeId> {
    let scene = ui.scene.borrow();
    let from = options[at];
    // The scroll view it is in (or that is inside it), else its own height.
    let own = scene.bounds(root).height;
    let page = std::iter::once(from)
        .chain(scene.ancestors(from))
        .find(|id| scene.is_scroll_content(*id))
        .and_then(|content| scene.parent(content))
        .map_or(own, |viewport| scene.bounds(viewport).height.min(own));
    let here = scene.bounds(from);
    if down {
        let limit = here.y + page;
        let last = options[at + 1..]
            .iter()
            .take_while(|id| {
                let r = scene.bounds(**id);
                r.y + r.height <= limit
            })
            .last();
        last.or(options.get(at + 1)).copied()
    } else {
        let limit = here.y + here.height - page;
        let first = options[..at]
            .iter()
            .rev()
            .take_while(|id| scene.bounds(**id).y >= limit)
            .last();
        first.or(at.checked_sub(1).map(|i| &options[i])).copied()
    }
}

/// `text` lowercased, at most 256 bytes; `None` with a control character.
pub(crate) fn bounded_prefix(text: &str) -> Option<String> {
    let mut result = String::new();
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_control() {
            return None;
        }
        if result.len() + character.len_utf8() > 256 {
            break;
        }
        result.push(character);
    }
    (!result.is_empty()).then_some(result)
}

/// Whether `label` starts with lowercased `prefix`, ignoring case.
pub(crate) fn matches_prefix(label: &str, prefix: &str) -> bool {
    let mut label = label.chars().flat_map(char::to_lowercase);
    prefix
        .chars()
        .all(|character| label.next() == Some(character))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typeahead_storage_is_bounded_with_unicode_case_expansion() {
        let prefix = bounded_prefix(&"İ".repeat(10000)).unwrap();
        assert!(prefix.len() <= 256);
        assert!(prefix.starts_with("i\u{307}"));
        assert!(matches_prefix(&"İ".repeat(10000), &prefix));
        assert!(bounded_prefix("\n").is_none());
    }
    #[test]
    fn a_prefix_jumps_and_a_repeated_letter_cycles() {
        let names = ["Apple", "Banana", "Blueberry", "Cherry"];
        let name = |i: usize| Some(names[i].to_string());
        let mut typed = Typeahead::default();
        assert_eq!(typed.find("b", 4, Some(0), name), Some(1));
        assert_eq!(typed.find("l", 4, Some(1), name), Some(2), "bl: Blueberry");
        let mut typed = Typeahead::default();
        assert_eq!(typed.find("b", 4, None, name), Some(1));
        assert_eq!(
            typed.find("b", 4, Some(1), name),
            Some(2),
            "b again: the next"
        );
        assert_eq!(typed.find("b", 4, Some(2), name), Some(1), "and round");
        assert_eq!(typed.find("z", 4, Some(1), name), None);
    }
}
