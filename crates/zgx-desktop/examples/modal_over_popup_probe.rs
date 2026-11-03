//! A modal opening over a popup shown in a window of its own (macOS): the
//! dropdown, taller than the window, is drawn in a popup window above it,
//! and the modal opening after it must close it rather than leave it drawn
//! over the modal. `scripts/macos_modal_over_popup_probe.py` runs it and
//! checks the lines.
//!
//! It prints `POPUPS <when> windows=<n> dropdown=<open> modal=<open>` with
//! the dropdown open and again after the modal opened (`windows`: zgx's
//! popup windows on screen and not faded out), then `MODAL_OVER_POPUP_DONE`.
use std::time::Duration;
use zgx::compose::prelude::*;
use zgx_desktop::{Application, WindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: "Modal over popup".into(),
            width: 420.,
            height: 220.,
            ..Default::default()
        })
        .run(|cx| {
            let dropdown = cx.ui.signal(false);
            let palette = cx.ui.signal(false);
            cx.render(
                column().p(20.).children([
                    popover(
                        "Dropdown",
                        dropdown.clone(),
                        button().size(100., 24.).child(text("Pick")),
                    )
                    // Taller than the window: shown in a popup window.
                    .w(220.)
                    .h(400.)
                    .child(text("Options")),
                    modal("Palette", palette.clone())
                        .w(300.)
                        .h(120.)
                        .child(text("Palette")),
                ]),
            );
            let window = cx.window.clone();
            cx.tasks.spawn(async move {
                let pause = |ms| zgx::timer::sleep(Duration::from_millis(ms));
                pause(1500).await;
                dropdown.set(true);
                pause(800).await;
                report("dropdown", &dropdown, &palette);
                palette.set(true);
                pause(800).await;
                report("modal", &dropdown, &palette);
                println!("MODAL_OVER_POPUP_DONE");
                window.close();
            });
        })
}

fn report(
    when: &str,
    dropdown: &zgx::reactive::Signal<bool>,
    palette: &zgx::reactive::Signal<bool>,
) {
    println!(
        "POPUPS {when} windows={} dropdown={} modal={}",
        shown::popup_windows(),
        dropdown.get(),
        palette.get()
    );
}

#[cfg(target_os = "macos")]
mod shown {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    /// zgx's popup windows (a `ZgxPopupView` in their content) on screen
    /// and not faded out.
    pub fn popup_windows() -> usize {
        let Some(mtm) = MainThreadMarker::new() else {
            return 0;
        };
        NSApplication::sharedApplication(mtm)
            .windows()
            .iter()
            .filter(|window| {
                // zgx's popup view is its panel's content's subview.
                window.isVisible()
                    && window.alphaValue() > 0.
                    && window.contentView().is_some_and(|content| {
                        content
                            .subviews()
                            .iter()
                            .any(|view| view.class().name().to_string_lossy() == "ZgxPopupView")
                    })
            })
            .count()
    }
}

#[cfg(not(target_os = "macos"))]
mod shown {
    pub fn popup_windows() -> usize {
        0
    }
}
