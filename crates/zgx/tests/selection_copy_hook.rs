//! A selectable region's copy hook (`View::on_selection_copy`): what a copy
//! out of it puts on the clipboard, by every route (`Ui::clipboard_shortcut`,
//! the selection's context menu, `Ui::copy_selection`), through one funnel
//! (`Ui::copy_selection_from`). The headless core's approximate layout puts
//! each grapheme in a cell 0.6 of the text size wide: at size 10, 6 wide.
use std::{cell::RefCell, rc::Rc};
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    platform::{FilePicker, Pending, Platform, PlatformService},
    scene::NodeId,
    widgets::{ClipboardContent, SelectionCopyRequest, Ui},
};

/// A clipboard in memory: what was written, text and HTML.
#[derive(Default)]
struct Clipboard(RefCell<Vec<(String, Option<String>)>>);
impl Platform for Clipboard {
    fn clipboard_text(&self) -> Option<String> {
        self.last().map(|(text, _)| text)
    }
    fn set_clipboard_text(&self, text: String) -> bool {
        self.0.borrow_mut().push((text, None));
        true
    }
    fn set_clipboard_rich_text(&self, text: String, html: String) -> bool {
        self.0.borrow_mut().push((text, Some(html)));
        true
    }
    fn open_files(&self, _: FilePicker) -> Pending<Result<Vec<std::path::PathBuf>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn save_file(&self, _: FilePicker) -> Pending<Result<Option<std::path::PathBuf>, String>> {
        Box::pin(async { Ok(None) })
    }
}
impl Clipboard {
    fn last(&self) -> Option<(String, Option<String>)> {
        self.0.borrow().last().cloned()
    }
}

/// A region's copy hook.
type Hook = Rc<dyn Fn(&SelectionCopyRequest) -> Option<ClipboardContent>>;
/// What a hook heard: the text, the HTML and the leaves of each copy.
type Heard = Rc<RefCell<Vec<(String, String, Vec<NodeId>)>>>;

/// A selectable region of `children` in a window with `clipboard`, and a
/// hook on it if `hook` is given.
fn region(
    ui: &mut Ui,
    clipboard: &Rc<Clipboard>,
    children: Vec<View>,
    hook: Option<Hook>,
) -> zgx::compose::ViewHandle {
    let mut region = column()
        .id("region")
        .w(300.)
        .gap(4.)
        .text_size(10.)
        .line_height(20.)
        .selectable(true)
        .children(children);
    if let Some(hook) = hook {
        region = region.on_selection_copy(move |request| hook(request));
    }
    let view = ui.mount(provide(
        PlatformService(clipboard.clone()),
        column().size(400., 300.).child(region),
    ));
    ui.prepare_frame();
    view
}
fn drag(ui: &mut Ui, from: (f32, f32), to: (f32, f32)) {
    let button = PointerButton::Primary;
    ui.dispatch(InputEvent::PointerDown {
        x: from.0,
        y: from.1,
        button,
    });
    ui.dispatch(InputEvent::PointerMove { x: to.0, y: to.1 });
    ui.dispatch(InputEvent::PointerUp {
        x: to.0,
        y: to.1,
        button,
    });
    ui.prepare_frame();
}
fn primary() -> Modifiers {
    Modifiers {
        meta: cfg!(target_os = "macos"),
        control: !cfg!(target_os = "macos"),
        ..Modifiers::default()
    }
}
/// A hook giving the selection's text upper-cased, its HTML wrapped, and
/// recording what it heard.
fn upper(heard: &Heard) -> Hook {
    let heard = heard.clone();
    Rc::new(move |request: &SelectionCopyRequest| {
        heard.borrow_mut().push((
            request.text.to_owned(),
            request.html.to_owned(),
            request.leaves.to_vec(),
        ));
        Some(ClipboardContent {
            text: request.text.to_uppercase(),
            html: Some(format!("<cite>{}</cite>", request.text)),
        })
    })
}

#[test]
fn every_route_writes_what_the_regions_hook_gives() {
    let clipboard = Rc::new(Clipboard::default());
    let heard = Heard::default();
    let mut ui = Ui::new(400., 300.);
    let view = region(
        &mut ui,
        &clipboard,
        vec![text("Copy these words")],
        Some(upper(&heard)),
    );
    drag(&mut ui, (0., 10.), (40., 10.));
    assert_eq!(ui.selected_text(), "Copy th");
    let given = Some((
        "COPY TH".to_owned(),
        Some("<cite>Copy th</cite>".to_owned()),
    ));
    // ⌘C (Ctrl elsewhere), as a window's key reaches it.
    let key = Key::Character("c".into());
    assert!(ui.clipboard_shortcut(&key, primary(), clipboard.as_ref()));
    assert_eq!(clipboard.last(), given);
    // The selection's context menu.
    let button = PointerButton::Secondary;
    ui.dispatch(InputEvent::PointerDown {
        x: 10.,
        y: 10.,
        button,
    });
    ui.dispatch(InputEvent::PointerUp {
        x: 10.,
        y: 10.,
        button,
    });
    ui.prepare_frame();
    let copy = view.find("selection-copy").expect("Copy shows");
    clipboard.0.borrow_mut().clear();
    ui.input.dispatch_to(&ui.scene, copy, InputEvent::Activate);
    assert_eq!(clipboard.last(), given);
    // From code, as the person would.
    clipboard.0.borrow_mut().clear();
    assert!(ui.copy_selection(&PlatformService(clipboard.clone())));
    assert_eq!(clipboard.last(), given);
    // Asked once a copy, with what the copy would have put there.
    let heard = heard.borrow();
    assert_eq!(heard.len(), 3);
    assert!(heard.iter().all(|(text, html, leaves)| text == "Copy th"
        && html.contains("Copy th")
        && leaves.len() == 1));
}

#[test]
fn a_hook_giving_nothing_keeps_the_selections_own_text_and_html() {
    let clipboard = Rc::new(Clipboard::default());
    let asked = Rc::new(std::cell::Cell::new(0));
    let counted = asked.clone();
    let mut ui = Ui::new(400., 300.);
    region(
        &mut ui,
        &clipboard,
        vec![text("Copy these words")],
        Some(Rc::new(move |_: &SelectionCopyRequest| {
            counted.set(counted.get() + 1);
            None
        })),
    );
    drag(&mut ui, (0., 10.), (40., 10.));
    let (text, html) = (ui.selected_text(), ui.selected_html());
    assert!(ui.copy_selection(&PlatformService(clipboard.clone())));
    assert_eq!(asked.get(), 1, "the hook was asked");
    assert_eq!(clipboard.last(), Some((text, Some(html))));
}

#[test]
fn a_region_without_a_hook_copies_its_selection_as_before() {
    let clipboard = Rc::new(Clipboard::default());
    let mut ui = Ui::new(400., 300.);
    region(&mut ui, &clipboard, vec![text("Copy these words")], None);
    drag(&mut ui, (0., 10.), (40., 10.));
    let (text, html) = (ui.selected_text(), ui.selected_html());
    let key = Key::Character("c".into());
    assert!(ui.clipboard_shortcut(&key, primary(), clipboard.as_ref()));
    assert_eq!(clipboard.last(), Some((text, Some(html))));
    // No selection: nothing copies, nothing is written.
    ui.clear_text_selection();
    clipboard.0.borrow_mut().clear();
    assert!(!ui.copy_selection(&PlatformService(clipboard.clone())));
    assert_eq!(clipboard.last(), None);
}

#[test]
fn the_leaves_are_the_text_nodes_spanned_in_document_order() {
    let clipboard = Rc::new(Clipboard::default());
    let heard = Heard::default();
    let mut ui = Ui::new(400., 300.);
    let view = region(
        &mut ui,
        &clipboard,
        vec![
            text("Alpha").id("a"),
            row()
                .gap(4.)
                .child(text("Beta").id("b"))
                .child(column().child(text("Gamma").id("c"))),
            text("Delta").id("d"),
        ],
        Some(upper(&heard)),
    );
    let region_node = view.find("region").unwrap();
    ui.select_all_text(region_node);
    ui.prepare_frame();
    assert!(ui.copy_selection(&PlatformService(clipboard.clone())));
    let ids = ["a", "b", "c", "d"].map(|id| view.find(id).unwrap());
    assert_eq!(heard.borrow()[0].2, ids, "each text node once, in order");
}

#[test]
fn the_hook_goes_with_its_region() {
    let clipboard = Rc::new(Clipboard::default());
    let kept = Rc::new(());
    let held = kept.clone();
    let mut ui = Ui::new(400., 300.);
    let view = region(
        &mut ui,
        &clipboard,
        vec![text("Copy these words")],
        Some(Rc::new(move |_: &SelectionCopyRequest| {
            let _ = &held;
            None
        })),
    );
    assert_eq!(Rc::strong_count(&kept), 2, "the region holds its hook");
    view.unmount();
    ui.prepare_frame();
    assert_eq!(Rc::strong_count(&kept), 1, "unmounted, the hook is let go");
}
