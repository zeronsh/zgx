//! A drag from outside the app reaching a window: files from a file
//! manager, text from another app (a browser's selection). The desktop host
//! hears them from the platform, and devtools drive them headless
//! (`zgx inspect files`, `zgx inspect text-drop`), both through what is here,
//! so the two can't drift.
use crate::{
    actions::Action,
    input::{DraggedText, InputEvent},
    widgets::Ui,
};
use std::{cell::RefCell, path::PathBuf};

thread_local! {
    /// What a drag of text over a window carries while it is: text, not yet
    /// known (X11 hands it over only as it drops).
    static HOVER: RefCell<Option<Action>> = const { RefCell::new(None) };
}

fn hover() -> Action {
    HOVER.with(|hover| {
        hover
            .borrow_mut()
            .get_or_insert_with(|| Action::new(DraggedText::new(String::new())))
            .clone()
    })
}

/// A drag carrying text from another app is over `ui` at `(x, y)` (its
/// points): whether it would drop there.
#[doc(hidden)]
pub fn text_over(ui: &Ui, x: f32, y: f32) -> bool {
    ui.input.drag_over(&ui.scene, &hover(), x, y)
}

/// That drag left `ui`, or was let go of where nothing took it.
#[doc(hidden)]
pub fn text_out(ui: &Ui) {
    if HOVER.with(|hover| hover.borrow_mut().take()).is_some() {
        ui.input.drag_out(&ui.scene);
    }
}

/// That drag was let go of over `ui` at `(x, y)`, carrying `text` and, where
/// the source offered it, the same as HTML (`DraggedText::html`): whether it
/// was taken.
#[doc(hidden)]
pub fn text_dropped(ui: &Ui, x: f32, y: f32, text: &str, html: Option<&str>) -> bool {
    HOVER.with(|hover| hover.borrow_mut().take());
    let text = Action::new(DraggedText::with_html(text, html.map(str::to_owned)));
    ui.input.drag_drop(&ui.scene, &text, x, y)
}

/// Files let go of over a window at `(x, y)`, in Linux's order: a
/// `FileDrop` for each path, then one `FilesDrop` with them all. (macOS and
/// Windows hand a drop over a file at a time and the desktop host sends
/// `FilesDrop` after the last, from a queue: the same events, but an app
/// shouldn't rely on their spacing.) `dispatch` delivers each event, and
/// whether a handler took it; `None` once the window has gone, which ends
/// the drop. Whether any handler took it.
#[doc(hidden)]
pub fn drop_files<E>(
    x: f32,
    y: f32,
    paths: std::sync::Arc<[PathBuf]>,
    mut dispatch: impl FnMut(InputEvent) -> Option<Result<bool, E>>,
) -> Result<bool, E> {
    let mut taken = false;
    for path in paths.iter() {
        match dispatch(InputEvent::FileDrop {
            x,
            y,
            path: path.clone(),
        }) {
            Some(result) => taken |= result?,
            None => return Ok(taken),
        }
    }
    if let Some(result) = dispatch(InputEvent::FilesDrop { x, y, paths }) {
        taken |= result?;
    }
    Ok(taken)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A drop is a `FileDrop` a file, then one `FilesDrop`, and taken if any
    /// was; a window gone mid-drop ends it there.
    #[test]
    fn files_drop_one_at_a_time_then_together_and_stop_as_the_window_goes() {
        let paths: std::sync::Arc<[PathBuf]> =
            [PathBuf::from("/a.txt"), PathBuf::from("/b.txt")].into();
        let mut heard = Vec::new();
        let taken = drop_files(4., 5., paths.clone(), |event| {
            let took = matches!(event, InputEvent::FilesDrop { .. });
            heard.push(event);
            Some(Ok::<_, ()>(took))
        });
        assert_eq!(taken, Ok(true));
        assert_eq!(heard.len(), 3);
        assert!(matches!(&heard[0], InputEvent::FileDrop { path, .. } if path == &paths[0]));
        assert!(matches!(&heard[1], InputEvent::FileDrop { path, .. } if path == &paths[1]));
        assert!(
            matches!(&heard[2], InputEvent::FilesDrop { x, y, paths: all } if *x == 4. && *y == 5. && all == &paths)
        );
        // Gone after the first: no more, and nothing taken.
        let mut calls = 0;
        let taken = drop_files(0., 0., paths, |_| {
            calls += 1;
            (calls == 1).then_some(Ok::<_, ()>(false))
        });
        assert_eq!((taken, calls), (Ok(false), 2));
    }

    /// Reduced motion set here reads over the system's until given back,
    /// and the system's reports go on underneath.
    #[test]
    fn reduced_motion_set_here_holds_over_the_systems_until_given_back() {
        let ui = Ui::new(100., 100.);
        let themes = ui.theme();
        let reported = |reduced_motion| crate::theme::SystemPreferences {
            reduced_motion,
            ..Default::default()
        };
        themes.set_system_preferences(reported(false));
        themes.override_reduced_motion(Some(true));
        assert!(themes.system_preferences().reduced_motion);
        // The system changes underneath: the override still holds.
        themes.set_system_preferences(reported(true));
        themes.override_reduced_motion(Some(false));
        assert!(!themes.system_preferences().reduced_motion);
        // Given back: the system's, as last reported.
        themes.override_reduced_motion(None);
        assert!(themes.system_preferences().reduced_motion);
    }
}
