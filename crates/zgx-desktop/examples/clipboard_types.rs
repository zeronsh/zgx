//! Types of an app's own through the system clipboard, where a web page's
//! copy keeps its own (Chromium's web custom data): Paste reads the
//! clipboard's `application/x-zgx-notes` and the types it holds; Copy puts
//! text, HTML and that type there in one change, for a browser's page or
//! another app to read (`scripts/clipboard_types_smoke.py`).
use std::{cell::RefCell, rc::Rc};
use zgx::{compose::prelude::*, platform::PlatformService};
use zgx_desktop::{Application, WindowOptions};

/// The type Copy writes and Paste reads, and what Copy writes.
const KIND: &str = "application/x-zgx-notes";
const COPIED: &str = "Copied <source id=\"s1\" /> 😀\n\nwith a blank line";
const COPIED_TEXT: &str = "Copied [1]";
const COPIED_HTML: &str = "<p>Copied [1]</p>";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: "zgx clipboard types".into(),
            width: 420.,
            height: 240.,
            ..Default::default()
        })
        .run(move |cx| {
            // What the window provides (its clipboard).
            let platform: Rc<RefCell<Option<Rc<PlatformService>>>> = Default::default();
            let found = platform.clone();
            let status = cx
                .ui
                .signal(String::from("Copy something typed, then Paste."));
            let paste = {
                let (platform, status) = (platform.clone(), status.clone());
                move || {
                    let platform = platform.borrow();
                    let (types, typed) = match platform.as_ref() {
                        Some(p) => (p.0.clipboard_types(), p.0.clipboard_typed(KIND)),
                        None => (Vec::new(), None),
                    };
                    println!("PASTED types={types:?} typed={typed:?}");
                    status.set(match &typed {
                        Some(typed) => format!("Pasted {} bytes of {KIND}", typed.len()),
                        None => format!("No {KIND} on the clipboard"),
                    });
                }
            };
            let copy = {
                let (platform, status) = (platform.clone(), status.clone());
                move || {
                    let types = [(KIND.to_owned(), COPIED.to_owned())];
                    let written = platform.borrow().as_ref().is_some_and(|p| {
                        let (text, html) = (COPIED_TEXT.into(), Some(COPIED_HTML.into()));
                        p.0.set_clipboard_typed(text, html, &types)
                    });
                    println!("COPIED written={written}");
                    status.set(format!("Copied typed: {written}"));
                }
            };
            cx.render(
                column()
                    .p(20.)
                    .gap(12.)
                    .child(component(move |cx| {
                        *found.borrow_mut() = cx.try_service::<PlatformService>();
                        div()
                    }))
                    .child(
                        row()
                            .gap(8.)
                            .child(button().w(120.).h(40.).child("Paste types").on_click(paste))
                            .child(button().w(120.).h(40.).child("Copy types").on_click(copy)),
                    )
                    .child(text_signal(move || status.get())),
            );
        })
}
