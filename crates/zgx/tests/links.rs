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
        .filter(|(_, s)| matches!(s.role, Role::Link | Role::Button) && !s.label.is_empty())
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
            .child(
                text_span("a link")
                    .href("https://example.org")
                    .on_click(move || *count.borrow_mut() += 1),
            )
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
fn a_run_that_only_acts_is_a_button_enter_and_space_press() {
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
    assert_eq!(
        ui.semantics.borrow().get(link).map(|s| s.role),
        Some(Role::Button)
    );
    key(&mut ui, Key::Enter, true);
    key(&mut ui, Key::Enter, false);
    assert_eq!(*clicks.borrow(), 1, "Enter presses it");
    key(&mut ui, Key::Space, true);
    key(&mut ui, Key::Space, false);
    assert_eq!(*clicks.borrow(), 2, "and Space, as a button's");
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

/// Each width a rich paragraph was shaped at for its links' geometry
/// (`set_rich_text_shaper`), shaping as the fallback layout does.
fn widths_shaped(ui: &Ui) -> Rc<RefCell<Vec<Option<f32>>>> {
    let widths = Rc::new(RefCell::new(Vec::new()));
    let seen = widths.clone();
    ui.scene
        .borrow_mut()
        .set_rich_text_shaper(move |rich, width| {
            seen.borrow_mut().push(width);
            Box::new(zgx::text_layout::FallbackTextLayout::with_rich(rich, width))
        });
    widths
}

/// The rich paragraph under `id`, depth first.
fn paragraph_under(ui: &Ui, id: NodeId) -> Option<NodeId> {
    let scene = ui.scene.borrow();
    if matches!(scene.kind(id), zgx::scene::NodeKind::RichText { .. }) {
        return Some(id);
    }
    let children = scene.children(id).to_vec();
    drop(scene);
    children
        .into_iter()
        .find_map(|child| paragraph_under(ui, child))
}

/// A link's boxes where they show: its own (what a screen reader's bounds
/// are) and each further line's.
fn link_rects(ui: &Ui, link: NodeId) -> Vec<zgx::scene::Rect> {
    let scene = ui.scene.borrow();
    let mut rects = vec![scene.bounds(link)];
    rects.extend(scene.children(link).iter().map(|&line| scene.bounds(line)));
    rects
}

/// Where the link's words are, shaped afresh at the paragraph's width.
fn words_at(ui: &Ui, paragraph: NodeId, range: std::ops::Range<usize>) -> Vec<zgx::scene::Rect> {
    use zgx::text_layout::TextLayout;
    let scene = ui.scene.borrow();
    let bounds = scene.bounds(paragraph);
    let zgx::scene::NodeKind::RichText { text } = scene.kind(paragraph) else {
        panic!("not a paragraph");
    };
    let layout = zgx::text_layout::FallbackTextLayout::with_rich(text, Some(bounds.width));
    layout
        .selection(range)
        .into_iter()
        .filter_map(|r| r.intersection(zgx::scene::Rect::new(0., 0., bounds.width, bounds.height)))
        .filter(|r| r.width > 0. && r.height > 0.)
        .map(|r| zgx::scene::Rect::new(bounds.x + r.x, bounds.y + r.y, r.width, r.height))
        .collect()
}

#[test]
fn a_links_boxes_follow_its_words_from_first_layout_and_none_are_shaped_before() {
    let mut ui = Ui::new(600., 400.);
    let shaped = widths_shaped(&ui);
    let width = ui.signal(400_f32);
    let height = ui.signal(60_f32);
    let (read_width, read_height) = (width.clone(), height.clone());
    let words = "Read the docs for the details of the layout below.";
    let _mounted = ui.mount(
        column().child(
            rich_text_signal(move || {
                vec![
                    text_span("Start: "),
                    text_span(words).href("https://example.org/docs"),
                ]
            })
            .text_wrap(true)
            .reactive_style(move || Styles::new().w(read_width.get()).h(read_height.get())),
        ),
    );
    let range = "Start: ".len().."Start: ".len() + words.len();
    let check = |ui: &mut Ui, when: &str| {
        let root = ui.scene.borrow().root();
        let paragraph = paragraph_under(ui, root).expect("the paragraph");
        let link = links(ui)[0];
        let rects = link_rects(ui, link);
        assert_eq!(
            rects,
            words_at(ui, paragraph, range.clone()),
            "{when}: the link's boxes are its words"
        );
        assert!(!rects.is_empty() && rects[0].width > 0., "{when}: placed");
        for r in &rects {
            let (x, y) = (r.x + r.width / 2., r.y + r.height / 2.);
            let hit = ui.scene.borrow().hit_test(x, y);
            assert!(
                hit == Some(link) || hit.is_some_and(|h| ui.scene.borrow().parent(h) == Some(link)),
                "{when}: {r:?}'s middle hits the link, not {hit:?}"
            );
            ui.dispatch(InputEvent::PointerMove { x, y });
            ui.prepare_frame();
            let hovered = ui.input.hovered();
            assert!(
                hovered == Some(link)
                    || hovered.is_some_and(|h| ui.scene.borrow().parent(h) == Some(link)),
                "{when}: the pointer over {r:?} is over the link, not {hovered:?}"
            );
        }
        // Away again: a hovered link's ring is the next step's to show or not.
        ui.dispatch(InputEvent::PointerMove { x: 590., y: 390. });
        ui.prepare_frame();
        rects
    };
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    assert!(
        !shaped.borrow().contains(&Some(0.)),
        "nothing shaped for links' boxes before the paragraph had a size: {:?}",
        shaped.borrow()
    );
    let wide = check(&mut ui, "laid out");
    width.set(120.);
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let narrow = check(&mut ui, "narrowed");
    assert!(
        narrow.len() > wide.len(),
        "its words wrap onto more lines: {wide:?} then {narrow:?}"
    );
    height.set(140.);
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    assert_eq!(
        check(&mut ui, "taller"),
        narrow,
        "a taller paragraph moves no word"
    );
}

#[test]
fn a_links_boxes_come_from_the_lines_laid_out_where_its_host_says() {
    use zgx::text_layout::TextLayout;
    let mut ui = Ui::new(600., 400.);
    let shaped = widths_shaped(&ui);
    let asked = Rc::new(RefCell::new(0));
    let count = asked.clone();
    ui.scene
        .borrow_mut()
        .set_rich_text_selection(move |rich, width, ranges| {
            *count.borrow_mut() += 1;
            let layout = zgx::text_layout::FallbackTextLayout::with_rich(rich, width);
            ranges
                .iter()
                .map(|range| layout.selection(range.clone()))
                .collect()
        });
    let width = ui.signal(400_f32);
    let read_width = width.clone();
    let words = "Read the docs for the details of the layout below.";
    let _mounted = ui.mount(
        column().child(
            rich_text_signal(move || {
                vec![
                    text_span("Start: "),
                    text_span(words).href("https://example.org/docs"),
                ]
            })
            .text_wrap(true)
            .reactive_style(move || Styles::new().w(read_width.get())),
        ),
    );
    let range = "Start: ".len().."Start: ".len() + words.len();
    for (step, w) in [(1, 400.), (2, 120.)] {
        width.set(w);
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        let root = ui.scene.borrow().root();
        let paragraph = paragraph_under(&ui, root).expect("the paragraph");
        let link = links(&ui)[0];
        assert_eq!(
            link_rects(&ui, link),
            words_at(&ui, paragraph, range.clone()),
            "at {w}"
        );
        assert_eq!(*asked.borrow(), step, "asked once a width, at {w}");
    }
    assert_eq!(*shaped.borrow(), [], "nothing shaped for the links' boxes");
}

#[test]
fn a_links_ring_moves_none_of_its_boxes() {
    let mut ui = Ui::new(600., 400.);
    let words = "Read the docs for the details of the layout below.";
    let _mounted = ui.mount(
        column().child(
            rich_text_signal(move || {
                vec![
                    text_span("Start: "),
                    text_span(words).href("https://example.org/docs"),
                ]
            })
            .text_wrap(true)
            .reactive_style(|| Styles::new().w(120.)),
        ),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let range = "Start: ".len().."Start: ".len() + words.len();
    let root = ui.scene.borrow().root();
    let paragraph = paragraph_under(&ui, root).expect("the paragraph");
    let link = links(&ui)[0];
    let ring = |ui: &Ui| match ui.scene.borrow().kind(link) {
        zgx::scene::NodeKind::Panel { quad, .. } => quad.border_width,
        // Not yet hovered or focused: no ring made.
        zgx::scene::NodeKind::Container(_) => 0.,
        other => panic!("a link's box, not {other:?}"),
    };
    let rest = link_rects(&ui, link);
    assert!(rest.len() > 2, "its words on several lines: {rest:?}");
    assert_eq!(rest, words_at(&ui, paragraph, range.clone()));
    assert_eq!(ring(&ui), 0.);
    // Under the pointer, then focused from the keyboard: its ring shows,
    // and every box stays on its words.
    let first = rest[0];
    ui.dispatch(InputEvent::PointerMove {
        x: first.x + first.width / 2.,
        y: first.y + first.height / 2.,
    });
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    assert_eq!(ring(&ui), 1., "hovered: its ring");
    assert_eq!(
        link_rects(&ui, link),
        rest,
        "hovered: its boxes where they were"
    );
    ui.dispatch(InputEvent::PointerMove { x: 590., y: 390. });
    ui.dispatch(InputEvent::KeyDown {
        key: Key::Tab,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    assert_eq!(ui.input.focused(), Some(link));
    assert_eq!(ring(&ui), 1., "tabbed to: its ring");
    assert_eq!(
        link_rects(&ui, link),
        rest,
        "focused: its boxes where they were"
    );
}
