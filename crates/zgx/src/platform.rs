//! What an app reaches on the machine beyond its window: the clipboard (text,
//! HTML and images) and the file dialogs. A desktop host provides it to the components it renders
//! (`compose::provide`, as `Context::try_service::<PlatformService>()`);
//! headless runs have none, and tests provide a fake.
use std::{future::Future, path::PathBuf, pin::Pin, rc::Rc, sync::Arc};

/// Work a dialog finishes later.
pub type Pending<T> = Pin<Box<dyn Future<Output = T>>>;

/// What a file dialog shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilePicker {
    /// The dialog's title.
    pub title: String,
    /// Where it opens.
    pub directory: Option<PathBuf>,
    /// The name a save dialog suggests.
    pub file_name: Option<String>,
    /// Filters: a name and its extensions, without the dot.
    pub filters: Vec<(String, Vec<String>)>,
    /// An open dialog lets the user pick several files.
    pub multiple: bool,
}

/// A notification the system shows (`Platform::notify`): in macOS's
/// Notification Center, as a Windows toast, through a Linux desktop's
/// notification server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Notification {
    /// Its title, in bold.
    pub title: String,
    /// The text under it.
    pub body: String,
    /// One posted with the same tag replaces it (a conversation's newest
    /// message, not one for each).
    pub tag: Option<String>,
}
impl Notification {
    /// A notification of `title` and `body`.
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            tag: None,
        }
    }
    /// One with the same tag replaces it.
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }
}
/// A folder a kept grant names (`Platform::resolve_folder`), its access
/// begun.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolvedFolder {
    /// Where it is now.
    pub path: PathBuf,
    /// The grant to keep instead, the old one gone stale: the folder moved
    /// or was renamed.
    pub renewed: Option<Vec<u8>>,
}
impl ResolvedFolder {
    /// The folder at `path`, its grant still current.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            renewed: None,
        }
    }
    /// With `grant` to keep instead of the stale one.
    pub fn renewed(mut self, grant: Vec<u8>) -> Self {
        self.renewed = Some(grant);
        self
    }
}

/// Why a notification was not shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotifyError {
    /// This host or system shows none: why, and what would (an app
    /// bundle on macOS, an app identity on Windows).
    Unsupported(String),
    /// The person has not allowed this app's notifications, or turned them
    /// off.
    Denied(String),
    /// The system refused it, saying why.
    Failed(String),
}
impl std::fmt::Display for NotifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotifyError::Unsupported(why) | NotifyError::Denied(why) | NotifyError::Failed(why) => {
                f.write_str(why)
            }
        }
    }
}

/// The clipboard, the system's URL handler and file dialogs, as a desktop
/// host provides them (see the module docs).
///
/// Most methods have a body that does without (no HTML on the clipboard, no
/// spelling, no notifications), so a host implements only what it has. A
/// wrapper around another platform (to log, to check a grant, to fake one
/// thing in a test) must forward every method, these defaulted ones too:
/// one it leaves out quietly does without, though the platform it wraps has
/// it. A wrapper that didn't forward `clipboard_html` lost every copy's
/// HTML. A test can list what a wrapper leaves out with `unforwarded`.
pub trait Platform {
    /// The clipboard's text, if it holds text.
    fn clipboard_text(&self) -> Option<String>;
    /// Put `text` on the clipboard; whether it took it.
    fn set_clipboard_text(&self, text: String) -> bool;
    /// The primary selection's text (X11's and Wayland's: what was last
    /// selected anywhere, which a middle click pastes). None where the
    /// platform has none.
    fn primary_text(&self) -> Option<String> {
        None
    }
    /// Make `text` the primary selection (a terminal's selection, as it
    /// settles); whether the platform has one and took it.
    fn set_primary_text(&self, text: String) -> bool {
        let _ = text;
        false
    }
    /// Open `url` with the system's handler for it (a browser for
    /// `https:`); whether the request was made. None opens nothing.
    fn open_url(&self, url: &str) -> bool {
        let _ = url;
        false
    }
    /// The misspelled words in `text`, as UTF-8 byte ranges, by the system's
    /// spelling checker (`View::spellcheck`). None checks nothing.
    fn misspellings(&self, text: &str) -> Vec<std::ops::Range<usize>> {
        let _ = text;
        Vec::new()
    }
    /// The checker's suggestions for misspelled `word`, best first.
    fn spelling_guesses(&self, word: &str) -> Vec<String> {
        let _ = word;
        Vec::new()
    }
    /// Add `word` to the user's dictionary (the menu's Learn Spelling).
    fn learn_spelling(&self, word: &str) {
        let _ = word;
    }
    /// Stop flagging `word` for the rest of the session (Ignore Spelling).
    fn ignore_spelling(&self, word: &str) {
        let _ = word;
    }
    /// What the system's substitutions would change in `text`, of the
    /// `kinds` asked for and the person has on (macOS's smart quotes and
    /// dashes, and their text replacements, in System Settings): each a
    /// UTF-8 byte range of `text` and what goes there. None changes nothing.
    fn substitutions(
        &self,
        text: &str,
        kinds: Substitutions,
    ) -> Vec<(std::ops::Range<usize>, String)> {
        let _ = (text, kinds);
        Vec::new()
    }
    /// The system's autocorrection for the word at `word` (a UTF-8 byte
    /// range of `text`, the rest its context), where the person has
    /// spelling correction on. None corrects nothing.
    fn correction(&self, text: &str, word: std::ops::Range<usize>) -> Option<String> {
        let _ = (text, word);
        None
    }
    /// The language the app's text is in (its i18n locale, `None` for the
    /// system's again), as CSS's `lang` on a document's root: Han
    /// characters take that language's fonts (a Japanese app's kanji a
    /// Japanese face, a traditional Chinese one's a traditional one), and
    /// every text shapes again. None does nothing.
    fn set_text_language(&self, language: Option<&str>) {
        let _ = language;
    }
    /// Add the TrueType or OpenType font in `bytes` to every window's text,
    /// beside the faces it has (another weight of a family it holds): text
    /// in its families shapes with it from now on, and every text measures,
    /// lays out and draws again. The family names it holds, or why not (not
    /// a font). None adds none.
    fn add_font(&self, bytes: Arc<[u8]>) -> Result<Vec<String>, String> {
        let _ = bytes;
        Err("no fonts are added here".into())
    }
    /// The fonts the app's windows shape text with beside the system's
    /// (bundled, or replaced since), each a TrueType or OpenType file, in
    /// the order they took them: what a renderer of its own installs to
    /// shape and draw as the windows do (`ui.render_canvas`). None here.
    fn fonts(&self) -> Vec<Arc<[u8]>> {
        Vec::new()
    }
    /// Whether the platform looks words up (`look_up`): macOS's dictionary
    /// panel. A context menu offers Look Up where it does.
    fn looks_up(&self) -> bool {
        false
    }
    /// Show the dictionary's entry for `text` over it, as AppKit's Look Up
    /// does: `baseline` is where its first character's baseline starts, in
    /// the window's points, and `size` its font size. Whether it showed.
    fn look_up(&self, text: &str, baseline: (f32, f32), size: f32) -> bool {
        let _ = (text, baseline, size);
        false
    }
    /// The files on the clipboard (copied in Finder, Explorer or a file
    /// manager), in order; none where it holds none. None reads none.
    fn clipboard_files(&self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
    /// Put `files` on the clipboard, as a file manager's copy does: for a
    /// test or a driven session (`zgx inspect clipboard set --files`) to
    /// paste them. Whether it took them; a clipboard of the process's own
    /// (`ZGX_CLIPBOARD=memory`) does, the system's does not.
    #[doc(hidden)]
    fn set_clipboard_files(&self, files: Vec<std::path::PathBuf>) -> bool {
        let _ = files;
        false
    }
    /// The clipboard's HTML, its rich form (a browser's or a word
    /// processor's copy: `text/html`, macOS's `public.html`, Windows'
    /// `HTML Format` fragment), if it holds some; its text is
    /// `clipboard_text`'s. None reads none.
    #[doc(hidden)]
    fn clipboard_html(&self) -> Option<String> {
        None
    }
    /// Put `text` on the clipboard with `html`, its rich form, where the
    /// platform takes both; plain text alone otherwise.
    fn set_clipboard_rich_text(&self, text: String, html: String) -> bool {
        let _ = html;
        self.set_clipboard_text(text)
    }
    /// The clipboard's image, if it holds one: a screenshot, an image
    /// editor's copy, a browser's Copy Image. Later, as decoding a large one
    /// takes a frame or more: a desktop host reads it off the UI thread.
    /// None reads none.
    fn clipboard_image(&self) -> Pending<Option<crate::image::ImageData>> {
        Box::pin(async { None })
    }
    /// Put `image` on the clipboard in the formats the platform's apps paste
    /// (its transform left out), encoded off the UI thread as reading is;
    /// whether it took it. None takes none.
    fn set_clipboard_image(&self, image: Arc<crate::image::ImageData>) -> Pending<bool> {
        let _ = image;
        Box::pin(async { false })
    }
    /// The files the user picks to open: none if they cancel.
    fn open_files(&self, picker: FilePicker) -> Pending<Result<Vec<PathBuf>, String>>;
    /// Where the user chooses to save: none if they cancel.
    fn save_file(&self, picker: FilePicker) -> Pending<Result<Option<PathBuf>, String>>;
    /// Ask for a folder: the one the user picks, `None` if they cancel. A
    /// platform without a folder dialog refuses.
    fn open_folder(&self, picker: FilePicker) -> Pending<Result<Option<PathBuf>, String>> {
        let _ = picker;
        Box::pin(async { Err("no folder dialog here".into()) })
    }
    /// A grant to `folder`, which the user picked, kept past this run: what
    /// to store, for `resolve_folder` on a later launch. macOS's is a
    /// security-scoped bookmark, which reaches the folder sandboxed or not
    /// and follows it moved or renamed. Its path by default (a UTF-8 one);
    /// None keeps none.
    fn bookmark_folder(&self, folder: &std::path::Path) -> Option<Vec<u8>> {
        folder.to_str().map(|path| path.as_bytes().to_vec())
    }
    /// The folder a kept grant names, if it still resolves to one, with its
    /// access begun until `release_folder`.
    fn resolve_folder(&self, grant: &[u8]) -> Option<ResolvedFolder> {
        let path = PathBuf::from(std::str::from_utf8(grant).ok()?);
        path.is_dir().then(|| ResolvedFolder::new(path))
    }
    /// The access `resolve_folder` began ends: the handle went.
    fn release_folder(&self, folder: &std::path::Path) {
        let _ = folder;
    }
    /// Show `notification`, settled once the system has it, or with why
    /// not. The first asks the person's permission where the system does
    /// (macOS). None shows none.
    fn notify(&self, notification: Notification) -> Pending<Result<(), NotifyError>> {
        let _ = notification;
        Box::pin(async {
            Err(NotifyError::Unsupported(
                "this host shows no system notifications".into(),
            ))
        })
    }
    /// The next notification the person clicks: its tag (as posted), the
    /// app brought forward first. None where clicks aren't told (a host
    /// without notifications).
    fn next_notification_click(&self) -> Pending<Option<String>> {
        Box::pin(async { None })
    }
    /// Put `badge` on the app's icon (the Dock's, the taskbar's, a Linux
    /// dock's), a count of what is unread; `None` takes it off. Whether the
    /// system shows one.
    fn set_badge(&self, badge: Option<&str>) -> bool {
        let _ = badge;
        false
    }
}

/// Which of the system's substitutions an editor asks for
/// (`Platform::substitutions`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Substitutions {
    /// Curly quotes and dashes for straight ones (`View::smart_punctuation`).
    pub punctuation: bool,
    /// The person's text replacements, "omw" for "On my way!"
    /// (`View::autocorrect`).
    pub replacements: bool,
}

/// The platform, as a service components find.
#[derive(Clone)]
pub struct PlatformService(pub Rc<dyn Platform>);

/// Asked before a link in text opens, on a click or its Open Link menu item,
/// where a view provides one (`compose::provide`): `false` leaves it unopened.
/// Told the URL.
#[derive(Clone)]
pub struct LinkGuard(pub Rc<dyn Fn(&str) -> bool>);

/// What a wrapper around a platform leaves unforwarded: each `Platform`
/// method `wrap`'s platform, called once, never hands to the one it wraps.
/// Each is called with nothing in it (an empty text, `about:blank`, no
/// badge): a wrapper that does a method's work itself does it then.
/// For a test of an app's wrapper, which should forward every one (see
/// `Platform`): `assert!(unforwarded(|inner| PlatformService(Rc::new(Mine(inner)))).is_empty())`.
#[doc(hidden)]
pub fn unforwarded(wrap: impl FnOnce(PlatformService) -> PlatformService) -> Vec<&'static str> {
    let probe = Rc::new(Probe::default());
    let wrapper = wrap(PlatformService(probe.clone())).0;
    let w = &*wrapper;
    let picker = FilePicker::default;
    let folder = std::path::Path::new("/");
    let image = crate::image::ImageData::new(1, 1, vec![0u8; 4]).expect("a 1x1 image");
    // Each call, in the trait's order; what a future holds isn't awaited:
    // a forwarding wrapper reaches the inner platform as it is called.
    let _ = w.clipboard_text();
    let _ = w.set_clipboard_text(String::new());
    let _ = w.primary_text();
    let _ = w.set_primary_text(String::new());
    let _ = w.open_url("about:blank");
    let _ = w.misspellings("");
    let _ = w.spelling_guesses("");
    w.learn_spelling("");
    w.ignore_spelling("");
    let _ = w.substitutions("", Substitutions::default());
    let _ = w.correction("", 0..0);
    w.set_text_language(None);
    let _ = w.looks_up();
    let _ = w.look_up("", (0., 0.), 0.);
    let _ = w.clipboard_files();
    let _ = w.set_clipboard_files(Vec::new());
    let _ = w.clipboard_html();
    let _ = w.set_clipboard_rich_text(String::new(), String::new());
    drop(w.clipboard_image());
    drop(w.set_clipboard_image(Arc::new(image)));
    drop(w.open_files(picker()));
    drop(w.save_file(picker()));
    drop(w.open_folder(picker()));
    let _ = w.bookmark_folder(folder);
    let _ = w.resolve_folder(&[]);
    w.release_folder(folder);
    drop(w.notify(Notification::new("", "")));
    drop(w.next_notification_click());
    let _ = w.set_badge(None);
    let heard = probe.heard.borrow();
    Probe::METHODS
        .iter()
        .copied()
        .filter(|method| !heard.contains(method))
        .collect()
}

/// A platform that notes each method called on it (`unforwarded`).
#[derive(Default)]
struct Probe {
    heard: std::cell::RefCell<Vec<&'static str>>,
}
impl Probe {
    /// Every `Platform` method, in the trait's order.
    const METHODS: [&'static str; 29] = [
        "clipboard_text",
        "set_clipboard_text",
        "primary_text",
        "set_primary_text",
        "open_url",
        "misspellings",
        "spelling_guesses",
        "learn_spelling",
        "ignore_spelling",
        "substitutions",
        "correction",
        "set_text_language",
        "looks_up",
        "look_up",
        "clipboard_files",
        "set_clipboard_files",
        "clipboard_html",
        "set_clipboard_rich_text",
        "clipboard_image",
        "set_clipboard_image",
        "open_files",
        "save_file",
        "open_folder",
        "bookmark_folder",
        "resolve_folder",
        "release_folder",
        "notify",
        "next_notification_click",
        "set_badge",
    ];
    fn heard(&self, method: &'static str) {
        self.heard.borrow_mut().push(method);
    }
}
impl Platform for Probe {
    fn clipboard_text(&self) -> Option<String> {
        self.heard("clipboard_text");
        None
    }
    fn set_clipboard_text(&self, _: String) -> bool {
        self.heard("set_clipboard_text");
        true
    }
    fn primary_text(&self) -> Option<String> {
        self.heard("primary_text");
        None
    }
    fn set_primary_text(&self, _: String) -> bool {
        self.heard("set_primary_text");
        true
    }
    fn open_url(&self, _: &str) -> bool {
        self.heard("open_url");
        true
    }
    fn misspellings(&self, _: &str) -> Vec<std::ops::Range<usize>> {
        self.heard("misspellings");
        Vec::new()
    }
    fn spelling_guesses(&self, _: &str) -> Vec<String> {
        self.heard("spelling_guesses");
        Vec::new()
    }
    fn learn_spelling(&self, _: &str) {
        self.heard("learn_spelling");
    }
    fn ignore_spelling(&self, _: &str) {
        self.heard("ignore_spelling");
    }
    fn substitutions(&self, _: &str, _: Substitutions) -> Vec<(std::ops::Range<usize>, String)> {
        self.heard("substitutions");
        Vec::new()
    }
    fn correction(&self, _: &str, _: std::ops::Range<usize>) -> Option<String> {
        self.heard("correction");
        None
    }
    fn set_text_language(&self, _: Option<&str>) {
        self.heard("set_text_language");
    }
    fn looks_up(&self) -> bool {
        self.heard("looks_up");
        true
    }
    fn look_up(&self, _: &str, _: (f32, f32), _: f32) -> bool {
        self.heard("look_up");
        true
    }
    fn clipboard_files(&self) -> Vec<PathBuf> {
        self.heard("clipboard_files");
        Vec::new()
    }
    fn set_clipboard_files(&self, _: Vec<PathBuf>) -> bool {
        self.heard("set_clipboard_files");
        true
    }
    fn clipboard_html(&self) -> Option<String> {
        self.heard("clipboard_html");
        None
    }
    fn set_clipboard_rich_text(&self, _: String, _: String) -> bool {
        self.heard("set_clipboard_rich_text");
        true
    }
    fn clipboard_image(&self) -> Pending<Option<crate::image::ImageData>> {
        self.heard("clipboard_image");
        Box::pin(async { None })
    }
    fn set_clipboard_image(&self, _: Arc<crate::image::ImageData>) -> Pending<bool> {
        self.heard("set_clipboard_image");
        Box::pin(async { true })
    }
    fn open_files(&self, _: FilePicker) -> Pending<Result<Vec<PathBuf>, String>> {
        self.heard("open_files");
        Box::pin(async { Ok(Vec::new()) })
    }
    fn save_file(&self, _: FilePicker) -> Pending<Result<Option<PathBuf>, String>> {
        self.heard("save_file");
        Box::pin(async { Ok(None) })
    }
    fn open_folder(&self, _: FilePicker) -> Pending<Result<Option<PathBuf>, String>> {
        self.heard("open_folder");
        Box::pin(async { Ok(None) })
    }
    fn bookmark_folder(&self, _: &std::path::Path) -> Option<Vec<u8>> {
        self.heard("bookmark_folder");
        None
    }
    fn resolve_folder(&self, _: &[u8]) -> Option<ResolvedFolder> {
        self.heard("resolve_folder");
        None
    }
    fn release_folder(&self, _: &std::path::Path) {
        self.heard("release_folder");
    }
    fn notify(&self, _: Notification) -> Pending<Result<(), NotifyError>> {
        self.heard("notify");
        Box::pin(async { Ok(()) })
    }
    fn next_notification_click(&self) -> Pending<Option<String>> {
        self.heard("next_notification_click");
        Box::pin(async { None })
    }
    fn set_badge(&self, _: Option<&str>) -> bool {
        self.heard("set_badge");
        true
    }
}

#[cfg(test)]
mod forwarding {
    use super::*;

    /// The probe lists every method the trait has: a method added to
    /// `Platform` must be added to `Probe` (and called in `unforwarded`) too.
    #[test]
    fn the_probe_knows_every_platform_method() {
        let source = include_str!("platform.rs");
        let start = source.find("pub trait Platform {").unwrap();
        let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
        let methods: Vec<&str> = body
            .lines()
            .filter_map(|line| line.strip_prefix("    fn "))
            .map(|rest| &rest[..rest.find(['(', '<']).unwrap()])
            .collect();
        assert_eq!(methods, Probe::METHODS);
    }

    /// A wrapper forwarding every method leaves none; one leaving out a
    /// defaulted method (as an app's left out `clipboard_html`) is named.
    #[test]
    fn a_wrapper_leaving_a_method_out_is_named() {
        assert!(unforwarded(|inner| inner).is_empty(), "the platform itself");
        struct Partial(PlatformService);
        impl Platform for Partial {
            fn clipboard_text(&self) -> Option<String> {
                self.0.0.clipboard_text()
            }
            fn set_clipboard_text(&self, text: String) -> bool {
                self.0.0.set_clipboard_text(text)
            }
            fn open_files(&self, picker: FilePicker) -> Pending<Result<Vec<PathBuf>, String>> {
                self.0.0.open_files(picker)
            }
            fn save_file(&self, picker: FilePicker) -> Pending<Result<Option<PathBuf>, String>> {
                self.0.0.save_file(picker)
            }
            fn set_clipboard_rich_text(&self, text: String, html: String) -> bool {
                self.0.0.set_clipboard_rich_text(text, html)
            }
        }
        let left = unforwarded(|inner| PlatformService(Rc::new(Partial(inner))));
        assert!(left.contains(&"clipboard_html"), "{left:?}");
        assert!(!left.contains(&"set_clipboard_rich_text"), "{left:?}");
        assert!(!left.contains(&"clipboard_text"), "{left:?}");
    }
}
