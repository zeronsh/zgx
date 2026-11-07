use std::{cell::RefCell, rc::Rc};
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    scene::NodeId,
    semantics::Role,
    widgets::Ui,
};

/// What a link asks of the host: the URLs it opened, the text it copied.
#[derive(Default)]
struct Host {
    opened: RefCell<Vec<String>>,
    clipboard: RefCell<Option<String>>,
}
impl zgx::platform::Platform for Host {
    fn clipboard_text(&self) -> Option<String> {
        self.clipboard.borrow().clone()
    }
    fn set_clipboard_text(&self, text: String) -> bool {
        *self.clipboard.borrow_mut() = Some(text);
        true
    }
    fn open_url(&self, url: &str) -> bool {
        self.opened.borrow_mut().push(url.to_owned());
        true
    }
    fn open_files(
        &self,
        _: zgx::platform::FilePicker,
    ) -> zgx::platform::Pending<Result<Vec<std::path::PathBuf>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn save_file(
        &self,
        _: zgx::platform::FilePicker,
    ) -> zgx::platform::Pending<Result<Option<std::path::PathBuf>, String>> {
        Box::pin(async { Ok(None) })
    }
}

/// The links shown, left to right.
fn links(ui: &Ui) -> Vec<NodeId> {
    let mut links: Vec<_> = ui
        .semantics
        .borrow()
        .iter()
        .filter(|(_, s)| s.role == Role::Link)
        .map(|(id, _)| id)
        .collect();
    links.sort_by(|a, b| {
        let scene = ui.scene.borrow();
        scene.bounds(*a).x.total_cmp(&scene.bounds(*b).x)
    });
    links
}

fn click(ui: &mut Ui, link: NodeId, button: PointerButton, held: Modifiers) {
    let r = ui.scene.borrow().bounds(link);
    let (x, y) = (r.x + r.width / 2., r.y + r.height / 2.);
    ui.dispatch_with_modifiers(InputEvent::PointerDown { x, y, button }, held);
    ui.dispatch_with_modifiers(InputEvent::PointerUp { x, y, button }, held);
    ui.prepare_frame();
}

const COMMAND: Modifiers = Modifiers {
    shift: false,
    control: !cfg!(target_os = "macos"),
    alt: false,
    meta: cfg!(target_os = "macos"),
};

#[test]
fn a_click_on_an_href_opens_it_and_tells_its_action_the_keys_held() {
    let host = Rc::new(Host::default());
    let held = Rc::new(RefCell::new(Vec::new()));
    let seen = held.clone();
    let mut ui = Ui::new(300., 100.);
    ui.mount(provide(
        zgx::platform::PlatformService(host.clone()),
        rich_text()
            .text_size(16.)
            .child(text_span("Read "))
            .child(
                text_span("the docs")
                    .href("https://example.org/docs")
                    .on_click_with(move |keys| seen.borrow_mut().push(keys)),
            )
            .child(text_span(" or "))
            .child(text_span("the site").href("https://example.org"))
            .into(),
    ));
    ui.prepare_frame();
    let links = links(&ui);
    assert_eq!(links.len(), 2, "an href alone makes a link");
    let r = ui.scene.borrow().bounds(links[1]);
    assert_eq!(
        ui.scene
            .borrow()
            .cursor_at(r.x + r.width / 2., r.y + r.height / 2.),
        Some(zgx::cursor::Cursor::Pointer),
        "a link's hand"
    );
    click(
        &mut ui,
        links[0],
        PointerButton::Primary,
        Modifiers::default(),
    );
    assert_eq!(*host.opened.borrow(), ["https://example.org/docs"]);
    assert_eq!(*held.borrow(), [Modifiers::default()]);
    // A ⌘-click (Control elsewhere): the action knows, to open it aside.
    click(&mut ui, links[0], PointerButton::Primary, COMMAND);
    assert_eq!(held.borrow()[1], COMMAND);
    // A middle click passes as one, as a browser's opens a background tab.
    click(
        &mut ui,
        links[0],
        PointerButton::Middle,
        Modifiers::default(),
    );
    assert_eq!(held.borrow()[2], COMMAND);
    assert_eq!(host.opened.borrow().len(), 3);
    // Activated from the keyboard or assistive technology, it opens too.
    ui.input
        .dispatch_to(&ui.scene, links[1], InputEvent::Activate);
    assert_eq!(host.opened.borrow()[3], "https://example.org");
}

#[test]
fn an_hrefs_context_menu_opens_or_copies_it_and_a_plain_links_has_none() {
    let host = Rc::new(Host::default());
    let outer = Rc::new(RefCell::new(0));
    let counted = outer.clone();
    let mut ui = Ui::new(300., 100.);
    let view = ui.mount(provide(
        zgx::platform::PlatformService(host.clone()),
        column()
            .size(300., 100.)
            .context_menu(move |_| {
                *counted.borrow_mut() += 1;
                vec![menu_item("Outer")]
            })
            .child(View::from(
                rich_text()
                    .text_size(16.)
                    .child(text_span("site").href("https://example.org/a"))
                    .child(text_span(" and "))
                    .child(text_span("action").on_click(|| {})),
            )),
    ));
    ui.prepare_frame();
    let links = links(&ui);
    click(
        &mut ui,
        links[0],
        PointerButton::Secondary,
        Modifiers::default(),
    );
    assert_eq!(*outer.borrow(), 0, "the link's menu, not the view's");
    let copy = view.find("link-copy").expect("Copy Link shows");
    ui.input.dispatch_to(&ui.scene, copy, InputEvent::Activate);
    assert_eq!(
        host.clipboard.borrow().as_deref(),
        Some("https://example.org/a")
    );
    assert!(host.opened.borrow().is_empty(), "copying opens nothing");
    click(
        &mut ui,
        links[0],
        PointerButton::Secondary,
        Modifiers::default(),
    );
    let open = view.find("link-open").expect("Open Link shows");
    ui.input.dispatch_to(&ui.scene, open, InputEvent::Activate);
    assert_eq!(*host.opened.borrow(), ["https://example.org/a"]);
    // A link without an href: the press goes on to the view's menu.
    click(
        &mut ui,
        links[1],
        PointerButton::Secondary,
        Modifiers::default(),
    );
    assert_eq!(*outer.borrow(), 1);
}

#[test]
fn only_web_and_mail_links_open_through_the_system() {
    // Text often isn't the app's own: a `file:` link would launch what it
    // names, another scheme its handler. Its action still runs.
    let host = Rc::new(Host::default());
    let acted = Rc::new(RefCell::new(0));
    let count = acted.clone();
    let mut ui = Ui::new(400., 100.);
    let view = ui.mount(provide(
        zgx::platform::PlatformService(host.clone()),
        rich_text()
            .text_size(16.)
            .child(
                text_span("app")
                    .href("file:///Applications/Calculator.app")
                    .on_click(move || *count.borrow_mut() += 1),
            )
            .child(text_span(" "))
            .child(text_span("share").href("smb://server/share"))
            .child(text_span(" "))
            .child(text_span("mail").href("MAILTO:someone@example.org"))
            .child(text_span(" "))
            .child(text_span("web").href("HTTPS://example.org"))
            .into(),
    ));
    ui.prepare_frame();
    let links = links(&ui);
    for link in &links {
        click(&mut ui, *link, PointerButton::Primary, Modifiers::default());
    }
    assert_eq!(*acted.borrow(), 1, "the action ran");
    assert_eq!(
        *host.opened.borrow(),
        ["MAILTO:someone@example.org", "HTTPS://example.org"]
    );
    // Its menu copies it, but doesn't open it.
    click(
        &mut ui,
        links[0],
        PointerButton::Secondary,
        Modifiers::default(),
    );
    assert!(view.find("link-copy").is_some());
    assert!(view.find("link-open").is_none(), "no Open Link for a file");
}

#[test]
fn a_links_focus_ring_shows_from_the_keyboard_not_a_click() {
    let mut ui = Ui::new(300., 100.);
    ui.mount(
        rich_text()
            .text_size(16.)
            .child(text_span("one").on_click(|| {}))
            .child(text_span(" and "))
            .child(text_span("two").on_click(|| {}))
            .into(),
    );
    ui.prepare_frame();
    let links = links(&ui);
    let ring = |ui: &Ui, link: NodeId| match ui.scene.borrow().kind(link) {
        zgx::scene::NodeKind::Panel { quad, .. } => quad.border_width,
        other => panic!("a link's box, not {other:?}"),
    };
    click(
        &mut ui,
        links[0],
        PointerButton::Primary,
        Modifiers::default(),
    );
    ui.dispatch(InputEvent::PointerMove { x: 290., y: 90. });
    ui.prepare_frame();
    assert_eq!(ui.input.focused(), Some(links[0]));
    assert_eq!(ring(&ui, links[0]), 0., "clicked: focused, no ring");
    ui.dispatch(InputEvent::KeyDown {
        key: Key::Tab,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    ui.prepare_frame();
    assert_eq!(ui.input.focused(), Some(links[1]));
    assert_eq!(ring(&ui, links[1]), 1., "tabbed to: its ring");
}

#[test]
fn a_link_follows_enter_as_it_goes_down_and_not_space() {
    let clicks = Rc::new(RefCell::new(0));
    let count = clicks.clone();
    let mut ui = Ui::new(300., 100.);
    ui.mount(
        rich_text()
            .text_size(16.)
            .child(text_span("a link").on_click(move || *count.borrow_mut() += 1))
            .into(),
    );
    ui.prepare_frame();
    let link = links(&ui)[0];
    ui.input.focus(&ui.scene, Some(link));
    let key = |ui: &mut Ui, key: Key, down: bool| {
        ui.dispatch(if down {
            InputEvent::KeyDown {
                key,
                modifiers: Modifiers::default(),
                repeat: false,
            }
        } else {
            InputEvent::KeyUp {
                key,
                modifiers: Modifiers::default(),
            }
        });
    };
    // Space scrolls the page in a browser; it follows no link.
    key(&mut ui, Key::Space, true);
    key(&mut ui, Key::Space, false);
    assert_eq!(*clicks.borrow(), 0);
    key(&mut ui, Key::Enter, true);
    assert_eq!(*clicks.borrow(), 1, "Enter follows it as it goes down");
    key(&mut ui, Key::Enter, false);
    assert_eq!(*clicks.borrow(), 1);
}

#[test]
fn a_spans_own_menu_opens_where_it_is_asked_and_a_run_with_only_one_is_no_link() {
    let host = Rc::new(Host::default());
    let mut ui = Ui::new(400., 120.);
    let view = ui.mount(provide(
        zgx::platform::PlatformService(host.clone()),
        rich_text()
            .id("para")
            .text_size(16.)
            .child(text_span("Note").context_menu(|_| vec![menu_item("Note menu").id("note")]))
            .child(text_span(" and "))
            .child(
                text_span("docs")
                    .href("https://example.org/docs")
                    .context_menu(|_| vec![menu_item("Docs menu").id("docs")]),
            )
            .into(),
    ));
    ui.prepare_frame();
    let links = links(&ui);
    assert_eq!(links.len(), 1, "a run with only a menu is no link");
    // At the pointer, on the run's text.
    let para = ui.scene.borrow().bounds(view.find("para").unwrap());
    let (x, y) = (para.x + 6., para.y + 8.);
    let button = PointerButton::Secondary;
    ui.dispatch(InputEvent::PointerDown { x, y, button });
    ui.dispatch(InputEvent::PointerUp { x, y, button });
    ui.prepare_frame();
    let note = view.find("note").expect("the run's own menu");
    let at = ui.scene.borrow().bounds(note);
    assert!(
        at.x >= x - 1. && at.x < x + 24. && at.y >= y - 1. && at.y < y + 24.,
        "at the pointer: {at:?} for {x},{y}"
    );
    assert!(view.find("link-copy").is_none());
    ui.dispatch(InputEvent::KeyDown {
        key: Key::Escape,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    ui.prepare_frame();
    // From the keyboard, a focused link's: under it, at its start.
    assert!(ui.input.focus(&ui.scene, Some(links[0])));
    ui.dispatch(InputEvent::KeyDown {
        key: Key::Function(10),
        modifiers: Modifiers {
            shift: true,
            ..Default::default()
        },
        repeat: false,
    });
    ui.prepare_frame();
    let docs = view
        .find("docs")
        .expect("the link's own menu, in place of its link menu");
    assert!(view.find("link-copy").is_none());
    let (link, at) = {
        let scene = ui.scene.borrow();
        (scene.bounds(links[0]), scene.bounds(docs))
    };
    assert!(
        (at.x - link.x).abs() < 24. && at.y >= link.y + link.height - 1.,
        "under the link: {at:?} for {link:?}"
    );
}

#[test]
fn a_click_that_says_it_handled_itself_keeps_the_href_shut() {
    let host = Rc::new(Host::default());
    let mut ui = Ui::new(300., 100.);
    ui.mount(provide(
        zgx::platform::PlatformService(host.clone()),
        rich_text()
            .text_size(16.)
            .child(
                text_span("mine")
                    .href("https://example.org/mine")
                    .on_click_handled(|_| true),
            )
            .child(text_span(" or "))
            .child(
                text_span("theirs")
                    .href("https://example.org/theirs")
                    .on_click_handled(|_| false),
            )
            .into(),
    ));
    ui.prepare_frame();
    let links = links(&ui);
    click(
        &mut ui,
        links[0],
        PointerButton::Primary,
        Modifiers::default(),
    );
    ui.input
        .dispatch_to(&ui.scene, links[0], InputEvent::Activate);
    assert!(host.opened.borrow().is_empty(), "handled: not opened");
    click(
        &mut ui,
        links[1],
        PointerButton::Primary,
        Modifiers::default(),
    );
    assert_eq!(*host.opened.borrow(), ["https://example.org/theirs"]);
}

#[test]
fn a_paragraphs_menu_told_the_span_hears_its_links_and_passes_them_on() {
    let host = Rc::new(Host::default());
    let told = Rc::new(RefCell::new(Vec::new()));
    let seen = told.clone();
    let mut ui = Ui::new(400., 100.);
    let view = ui.mount(provide(
        zgx::platform::PlatformService(host.clone()),
        View::from(
            rich_text()
                .text_size(16.)
                .child(text_span("Read "))
                .child(text_span("this").href("https://example.org/this"))
                .child(text_span(" not "))
                .child(text_span("that").href("https://example.org/that")),
        )
        .context_menu_with_span(move |_, span| {
            let span = span.cloned();
            seen.borrow_mut().push(span.clone());
            match span {
                Some(span) if &*span.text == "that" => Vec::new(),
                _ => vec![menu_item("Mine").id("mine")],
            }
        }),
    ));
    ui.prepare_frame();
    let links = links(&ui);
    click(
        &mut ui,
        links[0],
        PointerButton::Secondary,
        Modifiers::default(),
    );
    let span = told.borrow()[0].clone().expect("told the span");
    assert_eq!((span.index, &*span.text), (1, "this"));
    assert_eq!(span.href.as_deref(), Some("https://example.org/this"));
    assert!(view.find("mine").is_some() && view.find("link-copy").is_none());
    // Nothing back for a link: its own Open Link and Copy Link.
    ui.dispatch(InputEvent::KeyDown {
        key: Key::Escape,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    ui.prepare_frame();
    click(
        &mut ui,
        links[1],
        PointerButton::Secondary,
        Modifiers::default(),
    );
    assert_eq!(told.borrow()[1].as_ref().unwrap().index, 3);
    assert!(view.find("link-copy").is_some());
}

/// A paragraph with no link watches no box (each watched box is published
/// at every layout that moves it); given one, it watches its own and its
/// link is placed over its words, and follows them when the paragraph
/// narrows (its first line's piece, here).
#[test]
fn a_paragraph_watches_its_box_only_once_it_has_a_link() {
    let mut ui = Ui::new(600., 400.);
    let linked = ui.signal(false);
    let width = ui.signal(400_f32);
    let read = linked.clone();
    let read_width = width.clone();
    let _mounted = ui.mount(
        column().child(
            rich_text_signal(move || {
                let words = text_span("Read the docs for the details of the layout below.");
                match read.get() {
                    true => vec![text_span("Start: "), words.href("https://example.org/docs")],
                    false => vec![text_span("Start: "), words],
                }
            })
            .text_wrap(true)
            .reactive_style(move || Styles::new().w(read_width.get())),
        ),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    assert_eq!(ui.observer_counts()[2], 0, "no box watched without a link");
    linked.set(true);
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    assert!(
        ui.observer_counts()[2] > 0,
        "its box watched once it has a link"
    );
    let link = links(&ui)[0];
    let wide = ui.scene.borrow().bounds(link);
    assert!(wide.width > 0. && wide.height > 0.);
    width.set(120.);
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let narrow = ui.scene.borrow().bounds(link);
    assert!(
        narrow.width < wide.width,
        "the link's box follows its words as their line narrows: {wide:?} then {narrow:?}"
    );
}
