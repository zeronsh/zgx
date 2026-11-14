//! An editor's copies and cuts through the app (`View::on_copy`): the app says what goes on the clipboard,
//! text with HTML beside it, or nothing for the selection's plain text. Its context menu and
//! `EditorHandle::copy_to_clipboard` / `cut_to_clipboard` ask it (⌘C, ⌘X and the Edit menu, in zgx-desktop,
//! go through the same `Ui::copy_from`). A password field never asks it, nor does an empty selection.
//! The clipboard is this test's, in memory: never the machine's.
use std::{cell::RefCell, rc::Rc};
use zgx::{
    compose::{ViewHandle, prelude::*},
    input::{InputEvent, PointerButton},
    platform::PlatformService,
    widgets::{ClipboardContent, CopyRequest, EditorHandle, Ui},
};

/// What the clipboard holds: text, and HTML where given.
#[derive(Default)]
struct Clipboard(RefCell<Option<(String, Option<String>)>>);
impl zgx::platform::Platform for Clipboard {
    fn clipboard_text(&self) -> Option<String> {
        self.0.borrow().as_ref().map(|(text, _)| text.clone())
    }
    fn set_clipboard_text(&self, text: String) -> bool {
        *self.0.borrow_mut() = Some((text, None));
        true
    }
    fn set_clipboard_rich_text(&self, text: String, html: String) -> bool {
        *self.0.borrow_mut() = Some((text, Some(html)));
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

/// What the hook heard: each request's cut, range and text.
type Heard = Rc<RefCell<Vec<(bool, std::ops::Range<usize>, String)>>>;

struct Field {
    ui: Ui,
    view: ViewHandle,
    editor: EditorHandle,
    value: zgx::reactive::Signal<String>,
    clipboard: Rc<Clipboard>,
    platform: PlatformService,
    heard: Heard,
}

/// A text input of "Hello world" whose `on_copy` gives its selection upper-cased, with HTML, unless `plain`.
fn field(plain: bool) -> Field {
    let mut ui = Ui::new(400., 200.);
    let value = ui.signal("Hello world".to_owned());
    let heard: Heard = Rc::default();
    let hears = heard.clone();
    let clipboard = Rc::new(Clipboard::default());
    let platform = PlatformService(clipboard.clone());
    let view = ui.mount(provide(
        platform.clone(),
        column().size(400., 200.).child(
            text_input("Name", value.clone())
                .id("field")
                .w(300.)
                .h(32.)
                .on_copy(move |request: &CopyRequest| {
                    hears.borrow_mut().push((
                        request.cut,
                        request.range.clone(),
                        request.text.to_owned(),
                    ));
                    (!plain).then(|| ClipboardContent {
                        text: request.text.to_uppercase(),
                        html: Some(format!("<b>{}</b>", request.text)),
                    })
                }),
        ),
    ));
    ui.prepare_frame();
    let editor = ui.editor(view.find("field").unwrap()).unwrap();
    Field {
        ui,
        view,
        editor,
        value,
        clipboard,
        platform,
        heard,
    }
}

fn held(field: &Field) -> Option<(String, Option<String>)> {
    field.clipboard.0.borrow().clone()
}

fn context_menu_item(field: &mut Field, id: &str) {
    let bounds = field.ui.scene.borrow().bounds(field.editor.node);
    let (x, y) = (bounds.x + 4., bounds.y + bounds.height / 2.);
    let button = PointerButton::Secondary;
    field.ui.dispatch(InputEvent::PointerDown { x, y, button });
    field.ui.dispatch(InputEvent::PointerUp { x, y, button });
    field.ui.prepare_frame();
    let item = field.view.find(id).unwrap_or_else(|| panic!("{id} shows"));
    field
        .ui
        .input
        .dispatch_to(&field.ui.scene, item, InputEvent::Activate);
    field.ui.prepare_frame();
}

#[test]
fn copy_to_clipboard_puts_what_the_app_gives() {
    let f = field(false);
    f.editor.set_selection(6, 11);
    assert!(f.editor.copy_to_clipboard(&f.ui, &f.platform));
    assert_eq!(
        held(&f),
        Some(("WORLD".into(), Some("<b>world</b>".into())))
    );
    assert_eq!(*f.heard.borrow(), vec![(false, 6..11, "world".into())]);
    assert_eq!(f.value.get(), "Hello world", "a copy leaves the text");
    // Its readers are as they were: the selection's text, not the app's.
    assert_eq!(f.editor.copy(), "world");
}

#[test]
fn cut_to_clipboard_puts_it_and_deletes_the_selection_as_one_undo() {
    let f = field(false);
    f.editor.set_selection(6, 11);
    assert!(f.editor.cut_to_clipboard(&f.ui, &f.platform));
    assert_eq!(
        held(&f),
        Some(("WORLD".into(), Some("<b>world</b>".into())))
    );
    assert_eq!(*f.heard.borrow(), vec![(true, 6..11, "world".into())]);
    assert_eq!(f.value.get(), "Hello ");
    assert!(f.editor.undo());
    assert_eq!(f.value.get(), "Hello world", "one step brings it back");
}

#[test]
fn an_app_giving_nothing_copies_and_cuts_the_plain_text() {
    let f = field(true);
    f.editor.set_selection(0, 5);
    assert!(f.editor.copy_to_clipboard(&f.ui, &f.platform));
    assert_eq!(held(&f), Some(("Hello".into(), None)));
    assert!(f.editor.cut_to_clipboard(&f.ui, &f.platform));
    assert_eq!(held(&f), Some(("Hello".into(), None)));
    assert_eq!(f.value.get(), " world", "the cut deletes it all the same");
    assert_eq!(f.heard.borrow().len(), 2);
}

#[test]
fn the_context_menus_copy_and_cut_ask_the_app() {
    let mut f = field(false);
    f.editor.set_selection(0, 11);
    context_menu_item(&mut f, "editor-copy");
    assert_eq!(
        held(&f),
        Some(("HELLO WORLD".into(), Some("<b>Hello world</b>".into())))
    );
    assert_eq!(f.value.get(), "Hello world");
    f.editor.set_selection(0, 5);
    context_menu_item(&mut f, "editor-cut");
    assert_eq!(
        held(&f),
        Some(("HELLO".into(), Some("<b>Hello</b>".into())))
    );
    assert_eq!(f.value.get(), " world");
    assert_eq!(
        *f.heard.borrow(),
        vec![
            (false, 0..11, "Hello world".into()),
            (true, 0..5, "Hello".into())
        ]
    );
}

#[test]
fn a_password_field_never_asks_the_app_nor_copies() {
    let f = field(false);
    f.editor.set_secure(true);
    f.editor.set_selection(0, 11);
    assert!(!f.editor.copy_to_clipboard(&f.ui, &f.platform));
    assert!(!f.editor.cut_to_clipboard(&f.ui, &f.platform));
    assert!(
        !f.ui
            .copy_from(&f.editor, false, |_, _| panic!("nothing written"))
    );
    assert!(held(&f).is_none());
    assert!(f.heard.borrow().is_empty(), "the app never hears it");
    assert_eq!(f.value.get(), "Hello world", "nor does a cut delete it");
}

#[test]
fn an_empty_selection_asks_nothing() {
    let f = field(false);
    f.editor.set_selection(3, 3);
    assert!(!f.editor.copy_to_clipboard(&f.ui, &f.platform));
    assert!(!f.editor.cut_to_clipboard(&f.ui, &f.platform));
    assert!(held(&f).is_none());
    assert!(f.heard.borrow().is_empty());
}

#[test]
fn a_cut_in_a_read_only_editor_is_a_copy() {
    let f = field(false);
    f.editor.set_read_only(true);
    f.editor.set_selection(6, 11);
    assert!(f.editor.cut_to_clipboard(&f.ui, &f.platform));
    assert_eq!(*f.heard.borrow(), vec![(false, 6..11, "world".into())]);
    assert_eq!(f.value.get(), "Hello world");
}
