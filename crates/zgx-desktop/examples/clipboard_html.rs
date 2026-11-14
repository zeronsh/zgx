//! Rich text through the system clipboard: Paste reads the clipboard's HTML
//! (a browser's or a word processor's copy) and its plain text; Copy puts
//! HTML there with plain text beside it, for other apps
//! (`scripts/clipboard_html_smoke.py`).
use std::{cell::RefCell, rc::Rc};
use zgx::{compose::prelude::*, platform::PlatformService};
use zgx_desktop::{Application, WindowOptions};

/// What Copy puts on the clipboard: its HTML, and the text beside it.
const COPIED_HTML: &str = "<p>Copied <b>bold</b> and <i>italic</i></p>";
const COPIED_TEXT: &str = "Copied bold and italic";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: "zgx clipboard html".into(),
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
                .signal(String::from("Copy something rich, then Paste."));
            let paste = {
                let (platform, status) = (platform.clone(), status.clone());
                move || {
                    let platform = platform.borrow();
                    let (html, text) = match platform.as_ref() {
                        Some(p) => (p.0.clipboard_html(), p.0.clipboard_text()),
                        None => (None, None),
                    };
                    println!("PASTED html={html:?} text={text:?}");
                    status.set(match &html {
                        Some(html) => format!("Pasted {} bytes of HTML", html.len()),
                        None => "No HTML on the clipboard".into(),
                    });
                }
            };
            let copy = {
                let (platform, status) = (platform.clone(), status.clone());
                move || {
                    let written = platform.borrow().as_ref().is_some_and(|p| {
                        p.0.set_clipboard_rich_text(COPIED_TEXT.into(), COPIED_HTML.into())
                    });
                    println!("COPIED written={written}");
                    status.set(format!("Copied rich text: {written}"));
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
                            .child(button().w(120.).h(40.).child("Paste HTML").on_click(paste))
                            .child(button().w(120.).h(40.).child("Copy HTML").on_click(copy)),
                    )
                    .child(text_signal(move || status.get())),
            );
        })
}
