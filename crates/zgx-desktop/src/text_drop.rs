//! Text dragged in from another app (a browser's selection, an editor's)
//! drops into a field, as in AppKit's text views and a browser's fields:
//! the platform's drag reaches winit's windows
//! (`winit::platform::text_drop`), and the window's destinations hear it as
//! a drag from another of the app's windows carrying `DraggedText`
//! (`InputDispatcher::drag_over`, through `zgx::outside_drag`, which
//! `zgx inspect text-drop` drives too). What it carries is known only as it
//! drops (X11 hands it over then), so over a window it names no text.
//! Wayland's drags reach the data worker instead (`wayland_data`), which
//! asks the window's host the same (`over`, `out`, `dropped`). winit's hook
//! is the vendored winit's (`zgx_winit_patched`): with crates.io's, only
//! Wayland's drags drop.
#[cfg(zgx_winit_patched)]
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, Weak},
};
#[cfg(zgx_winit_patched)]
use winit::window::{Window, WindowId};

#[cfg(zgx_winit_patched)]
struct Target {
    ui: zgx::widgets::WeakUi,
    window: Weak<Window>,
    /// Wake the loop: a destination's change draws.
    wake: Box<dyn Fn()>,
}

#[cfg(zgx_winit_patched)]
thread_local! {
    /// The windows' interfaces, by their winit window.
    static WINDOWS: RefCell<HashMap<WindowId, Target>> = RefCell::default();
}

/// `window` shows `ui`: text dragged in from other apps drops there. The
/// first hands winit's windows the destinations.
#[cfg(zgx_winit_patched)]
pub(crate) fn add(window: &Arc<Window>, ui: &zgx::widgets::Ui, wake: impl Fn() + 'static) {
    let first = WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        windows.insert(
            window.id(),
            Target {
                ui: ui.downgrade(),
                window: Arc::downgrade(window),
                wake: Box::new(wake),
            },
        );
        windows.len() == 1
    });
    if first {
        winit::platform::text_drop::set_text_drops(Some(std::rc::Rc::new(Windows)));
    }
}

/// `window` closed.
#[cfg(zgx_winit_patched)]
pub(crate) fn remove(window: WindowId) {
    WINDOWS.with(|windows| windows.borrow_mut().remove(&window));
}

/// `window`'s interface, and the drag's point there (physical pixels) in
/// its coordinates.
#[cfg(zgx_winit_patched)]
fn at(
    window: WindowId,
    x: f64,
    y: f64,
    then: impl FnOnce(&zgx::widgets::Ui, f32, f32) -> bool,
) -> bool {
    let found = WINDOWS.with(|windows| {
        let windows = windows.borrow();
        let target = windows.get(&window)?;
        let scale = target
            .window
            .upgrade()
            .map_or(1., |window| window.scale_factor());
        Some((target.ui.upgrade()?, scale))
    });
    let Some((ui, scale)) = found else {
        return false;
    };
    let taken = then(&ui, (x / scale) as f32, (y / scale) as f32);
    wake(window);
    taken
}

#[cfg(zgx_winit_patched)]
fn wake(window: WindowId) {
    WINDOWS.with(|windows| {
        if let Some(target) = windows.borrow().get(&window) {
            (target.wake)();
        }
    });
}

/// A drag carrying text from another app is over `ui` at `(x, y)` (its
/// points): whether it would drop there. A drag of the app's own the
/// platform has (Windows asks its windows' drop targets of it too) is the
/// host's to carry to its windows: none here.
pub(crate) fn over(ui: &zgx::widgets::Ui, x: f32, y: f32) -> bool {
    !crate::native_drag::handed_off() && zgx::outside_drag::text_over(ui, x, y)
}

/// The drag left `ui`, or was let go of where nothing took it.
pub(crate) fn out(ui: &zgx::widgets::Ui) {
    zgx::outside_drag::text_out(ui);
}

/// The drag was let go of over `ui` at `(x, y)`, carrying `text` and, where
/// the source offered it, the same as HTML (`DraggedText::html`): whether it
/// was taken.
pub(crate) fn dropped(
    ui: &zgx::widgets::Ui,
    x: f32,
    y: f32,
    text: &str,
    html: Option<&str>,
) -> bool {
    if crate::native_drag::handed_off() {
        return false;
    }
    zgx::outside_drag::text_dropped(ui, x, y, text, html)
}

#[cfg(zgx_winit_patched)]
struct Windows;
#[cfg(zgx_winit_patched)]
impl winit::platform::text_drop::TextDrops for Windows {
    fn over(&self, window: WindowId, x: f64, y: f64) -> bool {
        at(window, x, y, over)
    }
    fn out(&self, window: WindowId) {
        at(window, 0., 0., |ui, _, _| {
            out(ui);
            false
        });
    }
    fn dropped(&self, window: WindowId, x: f64, y: f64, text: &str) -> bool {
        self.dropped_with_html(window, x, y, text, None)
    }
    fn dropped_with_html(
        &self,
        window: WindowId,
        x: f64,
        y: f64,
        text: &str,
        html: Option<&str>,
    ) -> bool {
        at(window, x, y, |ui, x, y| dropped(ui, x, y, text, html))
    }
}

#[cfg(all(test, zgx_winit_patched))]
mod tests {
    use super::*;
    use winit::platform::text_drop::TextDrops;
    use zgx::{compose::prelude::*, input::DraggedText};

    #[test]
    fn text_dragged_in_drops_into_the_field_under_it() {
        let mut ui = zgx::widgets::Ui::new(400., 300.);
        let value = ui.signal(String::from("ab"));
        let fixed = ui.signal(String::from("kept"));
        let view = ui.mount(
            column()
                .child(
                    text_input("Name", value.clone())
                        .id("field")
                        .size(200., 30.),
                )
                .child(
                    text_input("Fixed", fixed.clone())
                        .read_only(true)
                        .size(200., 30.),
                ),
        );
        ui.prepare_frame();
        // A placeholder id, never a window's; no winit window: scale 1.
        let window = WindowId::dummy();
        let woken = std::rc::Rc::new(std::cell::Cell::new(0));
        WINDOWS.with(|windows| {
            let woken = woken.clone();
            windows.borrow_mut().insert(
                window,
                Target {
                    ui: ui.downgrade(),
                    window: Weak::new(),
                    wake: Box::new(move || woken.set(woken.get() + 1)),
                },
            );
        });
        let drops = Windows;
        assert!(drops.over(window, 150., 15.), "a field takes text");
        assert!(!drops.over(window, 100., 45.), "a read-only one, none");
        assert!(drops.over(window, 150., 15.));
        drops.out(window);
        assert!(drops.over(window, 150., 15.));
        assert!(drops.dropped(window, 150., 15., " dropped"));
        ui.prepare_frame();
        assert_eq!(value.get(), "ab dropped");
        assert_eq!(fixed.get(), "kept");
        assert_eq!(ui.input.focused(), view.find("field"));
        assert_eq!(woken.get(), 6, "each wakes the loop to draw");
        remove(window);
        assert!(!drops.over(window, 150., 15.), "closed");
    }

    /// What another app offered as HTML beside its text (a browser's
    /// selection) reaches a destination with it, `DraggedText::html`; a plain
    /// text drag has none.
    #[test]
    fn text_dragged_in_carries_its_html() {
        let mut ui = zgx::widgets::Ui::new(400., 300.);
        // Each drop's text and HTML.
        type Dropped = Vec<(String, Option<String>)>;
        let got: std::rc::Rc<RefCell<Dropped>> = Default::default();
        let into = got.clone();
        ui.mount(
            div()
                .size(200., 100.)
                .on_drop(move |text: &DraggedText, _| {
                    into.borrow_mut()
                        .push((text.text().to_owned(), text.html().map(str::to_owned)));
                }),
        );
        ui.prepare_frame();
        let window = WindowId::dummy();
        WINDOWS.with(|windows| {
            windows.borrow_mut().insert(
                window,
                Target {
                    ui: ui.downgrade(),
                    window: Weak::new(),
                    wake: Box::new(|| {}),
                },
            );
        });
        let drops = Windows;
        assert!(drops.over(window, 50., 50.));
        assert!(drops.dropped_with_html(window, 50., 50., "bold", Some("<b>bold</b>")));
        assert!(drops.over(window, 50., 50.));
        assert!(drops.dropped(window, 50., 50., "plain"));
        assert_eq!(
            *got.borrow(),
            [
                ("bold".to_owned(), Some("<b>bold</b>".to_owned())),
                ("plain".to_owned(), None)
            ]
        );
        remove(window);
    }
}
