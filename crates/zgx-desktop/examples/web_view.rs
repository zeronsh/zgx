//! An embedded web page (WKWebView on macOS, WebView2 on Windows, WebKitGTK
//! on Linux) under zgx chrome: a toolbar with navigation, a frosted menu
//! popover over the page, and a floating frosted composer whose glass blurs
//! the page beneath it.
//!
//! `cargo run -p zgx-desktop --example web_view [-- URL]`
//! `-- --fixture`: checks geometry, layering, backdrops, hit testing and
//! script evaluation against live AppKit state, prints `FIXTURE` lines,
//! holds the window for a screenshot (`WINDOW <n>`, `screencapture -l <n>`),
//! then exits (non-zero on failure). `--zoom 1.25` zooms the window first:
//! the same checks, in zgx's points, with AppKit's frames at the zoom.
//! `--unaccelerated`: the page painted on the CPU (Linux;
//! `WebViewOptions::accelerated`). `--control`: commands read a line each
//! from stdin, so a probe can drive the page with no input sent:
//! `find <text>` (prints `FIND`), `save-link` (the page's script clicks its
//! download link), `open-download` (the app opens the same file itself), `reload` and `downloads save` or `downloads cancel` (the app's
//! download handler, as Ctrl+D sets it, or one cancelling each).
#[cfg(target_os = "macos")]
use std::time::Duration;
use std::{cell::RefCell, rc::Rc};
use zgx::{
    actions::{Action, KeyBinding, Keymap, Keystroke},
    compose::prelude::*,
    input::{EventPhase, InputEvent, Key, Modifiers},
    web_view::{WebView, WebViewEvent, WebViewOptions},
};
use zgx_desktop::{Application, MenuAction, WindowOptions};

/// The page's download link's file, for `--control`'s `open-download`.
const SAVED_FILE: &str = "data:application/octet-stream;base64,c2F2ZWQgYnkgdGhlIGFwcA==";
const PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>zgx web view</title>
<style>
  html, body { margin: 0; height: 100%; font: 17px -apple-system, system-ui, sans-serif; color: #fff; }
  body { background: linear-gradient(135deg, #ff4d6d 0%, #ffb347 30%, #3ddc97 60%, #3a86ff 100%); }
  main { padding: 48px 56px 160px; }
  h1 { font-size: 64px; margin: 0 0 12px; letter-spacing: -1px; text-shadow: 0 2px 12px rgba(0,0,0,.25); }
  .grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 18px; margin-top: 28px; }
  .tile { height: 120px; border-radius: 18px; display: flex; align-items: end; padding: 14px; font-weight: 700; font-size: 22px; }
  .stripes { height: 220px; margin-top: 28px; border-radius: 20px;
    background: repeating-linear-gradient(90deg, #111 0 18px, #fff 18px 36px); }
  p { max-width: 720px; line-height: 1.5; font-size: 19px; }
  button { font: inherit; padding: 10px 18px; border-radius: 12px; border: 0; background: #111; color: #fff; }
</style></head>
<body><main>
  <h1>Hello from the web</h1>
  <p>This page is the platform's web view, laid out by zgx. The menu and the composer are zgx
  content drawn above it; on macOS their glass blurs it.</p>
  <button onclick="window.zgx.postMessage({clicked: Date.now()})">Message the app</button>
  <input id="upload" type="file" multiple style="position:absolute;left:-9999px"
    onchange="window.chosen = [...this.files].map((f) => f.name).join(',')">
  <a id="save" download="zgx-example.txt" href="data:text/plain,saved by the app"
    style="position:absolute;left:700px;top:290px;color:#fff;font-weight:700">Save a file</a>
  <div class="grid">
    <div class="tile" style="background:#1d3557">Navy</div>
    <div class="tile" style="background:#e63946">Red</div>
    <div class="tile" style="background:#f1faee;color:#111">Snow</div>
    <div class="tile" style="background:#2a9d8f">Teal</div>
  </div>
  <div class="stripes"></div>
  <div class="grid">
    <div class="tile" style="background:#8338ec">Violet</div>
    <div class="tile" style="background:#ffbe0b;color:#111">Amber</div>
    <div class="tile" style="background:#000">Black</div>
    <div class="tile" style="background:#fb5607">Orange</div>
  </div>
</main></body></html>"#;

/// Whether `view` or one inside it is one of AppKit's print panel's.
#[cfg(target_os = "macos")]
fn holds_print_view(view: &objc2_app_kit::NSView) -> bool {
    view.class().name().to_string_lossy().contains("Print")
        || view.subviews().iter().any(|child| holds_print_view(&child))
}
/// A window's sheet, and the window, if one is attached (a page's dialog,
/// a print panel).
#[cfg(target_os = "macos")]
fn attached_sheet() -> Option<(
    objc2::rc::Retained<objc2_app_kit::NSWindow>,
    objc2::rc::Retained<objc2_app_kit::NSWindow>,
)> {
    let mtm = objc2::MainThreadMarker::new()?;
    objc2_app_kit::NSApplication::sharedApplication(mtm)
        .windows()
        .iter()
        .find_map(|window| Some((window.clone(), window.attachedSheet()?)))
}
/// End `sheet` with `button` (1000 the first, 1001 the second), typing
/// `typed` into its text field first: what its field held before, if it
/// has one.
#[cfg(target_os = "macos")]
fn answer_sheet(
    parent: &objc2_app_kit::NSWindow,
    sheet: &objc2_app_kit::NSWindow,
    button: isize,
    typed: Option<&str>,
) -> Option<String> {
    fn field(
        view: &objc2_app_kit::NSView,
    ) -> Option<objc2::rc::Retained<objc2_app_kit::NSTextField>> {
        if let Some(field) = view.downcast_ref::<objc2_app_kit::NSTextField>()
            && field.isEditable()
        {
            return Some(objc2::Message::retain(field));
        }
        view.subviews().iter().find_map(|child| field(&child))
    }
    let found = sheet.contentView().and_then(|view| field(&view));
    let held = found.as_ref().map(|field| field.stringValue().to_string());
    if let (Some(field), Some(typed)) = (&found, typed) {
        field.setStringValue(&objc2_foundation::NSString::from_str(typed));
    }
    parent.endSheet_returnCode(sheet, button);
    held
}

/// The fixture's file dialog: what a page's file input asks the window for
/// (`WebView::choose_files`), answered with one file at once, no panel shown.
#[derive(Default)]
struct FixtureFiles {
    asked: std::cell::Cell<Option<bool>>,
}
impl zgx::platform::Platform for FixtureFiles {
    fn clipboard_text(&self) -> Option<String> {
        None
    }
    fn set_clipboard_text(&self, _: String) -> bool {
        false
    }
    fn open_files(
        &self,
        picker: zgx::platform::FilePicker,
    ) -> zgx::platform::Pending<Result<Vec<std::path::PathBuf>, String>> {
        self.asked.set(Some(picker.multiple));
        let file = std::env::temp_dir().join("zgx-fixture-upload.txt");
        let written = std::fs::write(&file, "chosen for the page\n");
        Box::pin(async move { written.map(|()| vec![file]).map_err(|e| e.to_string()) })
    }
    fn save_file(
        &self,
        _: zgx::platform::FilePicker,
    ) -> zgx::platform::Pending<Result<Option<std::path::PathBuf>, String>> {
        Box::pin(async { Ok(None) })
    }
}
/// `view` with `files` for its file dialogs, if given (the fixture's).
fn provide_files(files: Option<Rc<FixtureFiles>>, view: View) -> View {
    match files {
        Some(files) => provide(zgx::platform::PlatformService(files), view),
        None => view,
    }
}

fn icon_button(label: &str) -> View {
    button()
        .h(30.)
        .px(10.)
        .rounded(8.)
        .bg(rgba(0xffffff14))
        .hover(|s| s.bg(rgba(0xffffff26)))
        .child(text(label).text_size(13.))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let fixture = args.iter().any(|arg| arg == "--fixture");
    let accelerated = !args.iter().any(|arg| arg == "--unaccelerated");
    let controlled = args.iter().any(|arg| arg == "--control");
    let zoom = args
        .iter()
        .skip_while(|arg| *arg != "--zoom")
        .nth(1)
        .and_then(|zoom| zoom.parse::<f64>().ok());
    // Not `--zoom`'s value.
    let url = args
        .iter()
        .find(|arg| !arg.starts_with("--") && arg.parse::<f64>().is_err())
        .cloned();
    Application::new()
        .window(WindowOptions {
            title: "zgx web view".into(),
            width: 1100.,
            height: 760.,
            ..Default::default()
        })
        .run(move |cx| {
            if let Some(zoom) = zoom {
                cx.ui.set_zoom(zoom);
            }
            let options = WebViewOptions {
                accelerated,
                ..Default::default()
            };
            let web = WebView::new(&cx.ui.runtime, options);
            let files = Rc::new(FixtureFiles::default());
            match &url {
                Some(url) => web.load_url(url.clone()),
                None => web.load_html(PAGE, Some("https://zgx.example/")),
            }
            let messages = cx.ui.signal(0_u32);
            let finished = cx.ui.signal(0_u32);
            let subscription = {
                let (messages, finished) = (messages.clone(), finished.clone());
                web.on_event(move |event| match event {
                    WebViewEvent::Message { body, .. } => {
                        println!("MESSAGE {body}");
                        messages.update(|count| *count += 1);
                    }
                    WebViewEvent::NavigationFinished { url } => {
                        println!("FINISHED {url}");
                        finished.update(|count| *count += 1);
                    }
                    WebViewEvent::NavigationFailed { url, error } => println!("FAILED {url}: {error}"),
                    WebViewEvent::TitleChanged { title } => println!("TITLE {title}"),
                    _ => {}
                })
            };
            let menu_open = cx.ui.signal(false);
            let page_shown = cx.ui.signal(true);
            let draft = cx.ui.signal(String::new());
            let toolbar = {
                let (back, forward, reload) = (web.clone(), web.clone(), web.clone());
                let (title, address) = (web.clone(), web.clone());
                let anchor = icon_button("Menu").id("menu-button").on_click({
                    let open = menu_open.clone();
                    move || {
                        open.set(!open.get());
                    }
                });
                let menu = popover("Page menu", menu_open.clone(), anchor)
                    .id("menu")
                    .w(240.)
                    .p(8.)
                    .gap(2.)
                    .rounded(14.)
                    .blur(24.)
                    .bg(rgba(0x1c1d2299))
                    .shadow(zgx::scene::BoxShadow {
                        color: rgba(0x00000066),
                        offset: zgx::scene::Transform { x: 0., y: 12. },
                        blur_radius: 16.,
                        spread: 0.,
                    })
                    .border(1.)
                    .border_color(rgba(0xffffff2a))
                    .children(["Reload page", "Open Web Inspector", "Zoom to 125%", "Reset zoom"].map(
                        |label| {
                            let web = web.clone();
                            let open = menu_open.clone();
                            let label_owned = label.to_string();
                            button()
                                .h(32.)
                                .px(10.)
                                .rounded(8.)
                                .bg(rgba(0x00000000))
                                .border(0.)
                                .items_start()
                                .hover(|s| s.bg(rgba(0xffffff1c)))
                                .child(text(label).text_size(13.))
                                .on_click(move || {
                                    match label_owned.as_str() {
                                        "Reload page" => web.reload(),
                                        "Open Web Inspector" => web.open_devtools(),
                                        "Zoom to 125%" => web.set_zoom(1.25),
                                        _ => web.set_zoom(1.),
                                    }
                                    open.set(false);
                                })
                        },
                    ));
                row()
                    .w_full()
                    .h(48.)
                    .px(12.)
                    .gap(6.)
                    .items_center()
                    .child(icon_button("‹").on_click(move || back.go_back()))
                    .child(icon_button("›").on_click(move || forward.go_forward()))
                    .child(icon_button("Reload").on_click(move || reload.reload()))
                    .child(
                        column()
                            .grow()
                            .flex_shrink(1.)
                            .min_w(0.)
                            .px(10.)
                            .child(text_signal(move || title.title()).text_size(13.))
                            .child(
                                text_signal(move || {
                                    let state = address.state().get();
                                    if state.loading {
                                        format!("Loading {:.0}% · {}", state.progress * 100., state.url)
                                    } else {
                                        state.url
                                    }
                                })
                                .text_size(11.)
                                .text_color(rgba(0xffffff88)),
                            ),
                    )
                    .child(menu)
            };
            let composer = {
                let send = web.clone();
                let draft_value = draft.clone();
                column()
                    .absolute()
                    .left(0.)
                    .right(0.)
                    .bottom(28.)
                    .items_center()
                    .child(
                        row()
                            .id("composer")
                            .w(600.)
                            .h(64.)
                            .px(14.)
                            .gap(10.)
                            .items_center()
                            .rounded(22.)
                            .blur(22.)
                            .bg(rgba(0x16171c80))
                            .border(1.)
                            .border_color(rgba(0xffffff30))
                            .child(text_input("Message", draft.clone()).grow().h(40.))
                            .child(
                                button()
                                    .id("send")
                                    .h(36.)
                                    .px(16.)
                                    .rounded(12.)
                                    .bg(rgb(0x3a86ff))
                                    .child(text("Send to page").text_size(13.))
                                    .on_click(move || {
                                        let text = draft_value.get();
                                        let escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
                                        send.run_js(format!(
                                            "document.querySelector('h1').textContent = '{escaped}' || 'Hello from the web'"
                                        ));
                                    }),
                            ),
                    )
            };
            let page = overlay()
                .id("pane")
                .w_full()
                .grow()
                .flex_shrink(1.)
                .min_h(0.)
                .child(provide_files(
                    fixture.then(|| files.clone()),
                    web_view(&web)
                        .id("page")
                        .absolute()
                        .left(12.)
                        .right(12.)
                        .top(0.)
                        .bottom(12.)
                        .rounded(14.)
                        .bg(rgb(0x202228))
                        .reactive_style({
                            let shown = page_shown.clone();
                            move || if shown.get() { Styles::default() } else { Styles::default().hidden() }
                        }),
                ))
                .child(composer);
            let view = cx.render(
                column()
                    .w_full()
                    .h_full()
                    .bg(rgb(0x141519))
                    .text_color(rgb(0xeceef4))
                    .child(toolbar)
                    .child(page),
            );
            // Ctrl+F: find the heading's first word, printing `FIND`. Pressed
            // in the page (which leaves it alone), it reaches this binding:
            // WebView2's own find key is off.
            // Ctrl+D (reserved): the app saves the page's downloads into
            // `ZGX_EXAMPLE_SAVE_DIR` (a temporary folder without it),
            // printing `DOWNLOAD <name> <type>` and `DOWNLOAD_STATUS …`.
            // Before it the app has no handler, and saves none.
            let root = cx.ui.root();
            let finder = web.clone();
            cx.ui.on_event(root, false, move |event| {
                if event.phase != EventPhase::Capture
                    && let InputEvent::Action(action) = &event.event
                    && action.downcast_ref::<MenuAction>().is_some_and(|action| action.0 == "find")
                {
                    find(&finder, "Hello");
                    event.prevent_default();
                }
            });
            let downloads = web.clone();
            cx.ui.on_event(root, false, move |event| {
                if event.phase != EventPhase::Capture
                    && let InputEvent::Action(action) = &event.event
                    && action.downcast_ref::<MenuAction>().is_some_and(|action| action.0 == "downloads")
                {
                    handle_downloads(&downloads, true);
                    event.prevent_default();
                }
            });
            let control = Modifiers {
                control: true,
                ..Default::default()
            };
            let keymap = cx.ui.input.bind_keys(
                root,
                Keymap::new()
                    .bind(
                        KeyBinding::new(
                            [Keystroke::new(Key::Character("f".into()), control)],
                            Action::new(MenuAction("find".into())),
                        )
                        .expect("one keystroke"),
                    )
                    .bind(
                        KeyBinding::new(
                            [Keystroke::new(Key::Character("d".into()), control)],
                            Action::new(MenuAction("downloads".into())),
                        )
                        .expect("one keystroke")
                        .reserved(),
                    ),
            );
            let keep = Rc::new(RefCell::new(Some((subscription, keymap))));
            cx.on_closed(move || drop(keep.borrow_mut().take()));
            if controlled {
                follow_commands(cx, web.clone());
            }
            if fixture {
                run_fixture(
                    cx,
                    web,
                    Fixture {
                        view,
                        menu_open,
                        page_shown,
                        finished,
                        messages,
                        files,
                    },
                );
            }
        })
}

/// Find `text` in the page, printing `FIND <text> found=… matches=…`.
fn find(web: &WebView, text: &str) {
    let shown = text.to_owned();
    web.find(text, Default::default(), move |found| {
        println!(
            "FIND {shown} found={} matches={:?}",
            found.found, found.matches
        )
    });
}

/// The app's download handler: each download saved into
/// `ZGX_EXAMPLE_SAVE_DIR` (a temporary folder without it), printing
/// `DOWNLOAD <name> <type>` and `DOWNLOAD_STATUS …`, or, unless `save`,
/// each cancelled (`DOWNLOAD_CANCELLED <name>`).
fn handle_downloads(web: &WebView, save: bool) {
    let folder = std::env::var_os("ZGX_EXAMPLE_SAVE_DIR")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    web.set_download_handler(move |request, responder| {
        println!("DOWNLOAD {} {}", request.name, request.mime);
        if save {
            responder.save_to(folder.join(&request.name), |status| {
                println!("DOWNLOAD_STATUS {status:?}")
            });
        } else {
            println!("DOWNLOAD_CANCELLED {}", request.name);
            responder.cancel();
        }
    });
    println!("DOWNLOADS {}", if save { "on" } else { "cancelled" });
}

/// `--control`: a command a line from stdin (see the module documentation),
/// read on a thread of its own and carried out on the UI thread.
fn follow_commands(cx: &mut zgx_desktop::WindowContext, web: WebView) {
    let (send, commands) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lines().map_while(Result::ok) {
            if send.send(line).is_err() {
                break;
            }
        }
    });
    cx.tasks.spawn(async move {
        loop {
            zgx::timer::sleep(std::time::Duration::from_millis(50)).await;
            while let Ok(line) = commands.try_recv() {
                let (command, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
                match command {
                    "find" => find(&web, rest),
                    "save-link" => web.run_js("document.getElementById('save').click()"),
                    // The same file, opened by the app (a browser's own
                    // navigation, no page script): a download WebView2
                    // starts with no gesture to ask for.
                    "open-download" => web.load_url(SAVED_FILE),
                    "reload" => web.reload(),
                    "downloads" => handle_downloads(&web, rest == "save"),
                    _ => println!("CONTROL unknown: {line}"),
                }
            }
        }
    });
}

/// What the fixture reads and drives besides the window and the page.
struct Fixture {
    view: zgx::compose::ViewHandle,
    menu_open: zgx::reactive::Signal<bool>,
    page_shown: zgx::reactive::Signal<bool>,
    finished: zgx::reactive::Signal<u32>,
    messages: zgx::reactive::Signal<u32>,
    files: Rc<FixtureFiles>,
}

/// Checks against live AppKit state (see the module documentation).
fn run_fixture(cx: &mut zgx_desktop::WindowContext, web: WebView, fixture: Fixture) {
    let Fixture {
        view,
        menu_open,
        page_shown,
        finished,
        messages,
        files,
    } = fixture;
    #[cfg(target_os = "macos")]
    {
        let scene = cx.ui.scene.clone();
        let window = cx.window.clone();
        let diagnostics = cx.native_diagnostics();
        let ui_input = cx.ui.input.clone();
        let runtime = cx.ui.runtime.clone();
        cx.tasks.spawn(async move {
            let mut failures = 0;
            let mut check = |name: &str, ok: bool, detail: String| {
                println!(
                    "FIXTURE {} {name}: {detail}",
                    if ok { "ok" } else { "FAIL" }
                );
                if !ok {
                    failures += 1;
                }
            };
            let sleep = |ms| zgx::timer::sleep(Duration::from_millis(ms));
            // Equal but for float rounding (AppKit's points over a zoom).
            let close = |a: zgx::scene::Rect, b: zgx::scene::Rect| {
                [a.x - b.x, a.y - b.y, a.width - b.width, a.height - b.height]
                    .iter()
                    .all(|d| d.abs() < 0.01)
            };
            let framed = |frame: Option<zgx::scene::Rect>, b| frame.is_some_and(|a| close(a, b));
            let cut_to = |masks: &[(zgx::scene::Rect, f64)], b, radius: f64| {
                masks.len() == 1 && close(masks[0].0, b) && (masks[0].1 - radius).abs() < 0.01
            };
            let at = |point: Option<(f32, f32)>, (x, y): (f32, f32)| {
                point.is_some_and(|(px, py)| (px - x).abs() < 0.01 && (py - y).abs() < 0.01)
            };
            for _ in 0..100 {
                if finished.with_untracked(|count| *count) > 0 {
                    break;
                }
                sleep(50).await;
            }
            sleep(300).await;
            check(
                "page loaded",
                finished.with_untracked(|count| *count) > 0,
                format!("{} finishes", finished.with_untracked(|count| *count)),
            );
            let node = |id: &str| view.find(id).unwrap();
            let bounds = |id: &str| scene.borrow().bounds(node(id));
            let page = node("page");
            let natives = diagnostics.natives();
            let report = natives.iter().find(|report| report.node == page).cloned();
            check("page view exists", report.is_some(), format!("{natives:?}"));
            if let Some(report) = &report {
                check(
                    "page frame follows layout",
                    framed(report.frame, bounds("page")),
                    format!("{:?} vs {:?}", report.frame, bounds("page")),
                );
                check(
                    "page visible",
                    !report.hidden && report.alpha == 1.,
                    format!("hidden={} alpha={}", report.hidden, report.alpha),
                );
                check(
                    "page cut to its own rounded corners",
                    cut_to(&report.masks[..], bounds("page"), 14.),
                    format!("{:?}", report.masks),
                );
            }
            // Script evaluation round trip.
            let reply = Rc::new(RefCell::new(None));
            {
                let reply = reply.clone();
                web.evaluate_js("document.title + ' ' + (6 * 7)", move |value| {
                    *reply.borrow_mut() = Some(value)
                });
            }
            for _ in 0..40 {
                if reply.borrow().is_some() {
                    break;
                }
                sleep(25).await;
            }
            let value = reply.borrow().clone();
            check(
                "evaluate_js",
                value == Some(Ok("\"zgx web view 42\"".into())),
                format!("{value:?}"),
            );
            // A zoomed window's page is zoomed as much: its CSS width is
            // its box's in zgx's points, whatever AppKit's frame is.
            let width = Rc::new(RefCell::new(None));
            {
                let width = width.clone();
                web.evaluate_js("window.innerWidth", move |value| {
                    *width.borrow_mut() = Some(value)
                });
            }
            for _ in 0..40 {
                if width.borrow().is_some() {
                    break;
                }
                sleep(25).await;
            }
            let css = width
                .borrow()
                .clone()
                .and_then(|value| value.ok())
                .and_then(|value| value.parse::<f32>().ok());
            check(
                "page CSS width is its box's",
                css.is_some_and(|css| (css - bounds("page").width).abs() <= 1.),
                format!("{css:?} vs {}", bounds("page").width),
            );
            // Script dialogs: sheets on the window, answered here as a
            // person would (OK, OK, Cancel, then a name typed and OK).
            web.run_js(
                "window.dialogs = []; dialogs.push(alert('Hello from the page') === undefined); \
                 dialogs.push(confirm('Sure?')); dialogs.push(confirm('Again?')); \
                 dialogs.push(prompt('Name?', 'default'));",
            );
            let mut shown = Vec::new();
            for (button, typed) in [
                (1000, None),
                (1000, None),
                (1001, None),
                (1000, Some("zgx")),
            ] {
                let mut sheet = None;
                for _ in 0..60 {
                    sleep(50).await;
                    sheet = attached_sheet();
                    if sheet.is_some() {
                        break;
                    }
                }
                let Some((parent, sheet)) = sheet else {
                    break;
                };
                shown.push(answer_sheet(&parent, &sheet, button, typed));
                sleep(100).await;
            }
            let dialogs = Rc::new(RefCell::new(None));
            {
                let dialogs = dialogs.clone();
                web.evaluate_js("JSON.stringify(window.dialogs)", move |value| {
                    *dialogs.borrow_mut() = Some(value)
                });
            }
            for _ in 0..40 {
                if dialogs.borrow().is_some() {
                    break;
                }
                sleep(25).await;
            }
            let dialogs = dialogs.borrow().clone();
            check(
                "alert, confirm and prompt are sheets with their answers",
                shown.len() == 4
                    && shown[3] == Some("default".into())
                    && dialogs == Some(Ok("\"[true,true,false,\\\"zgx\\\"]\"".into())),
                format!("prompt fields {shown:?}, page heard {dialogs:?}"),
            );
            // A file input: the window's file dialog (the fixture's here,
            // which shows no panel), its file the input's.
            web.run_js("document.getElementById('upload').click()");
            let chosen = Rc::new(RefCell::new(None));
            for _ in 0..40 {
                sleep(50).await;
                let reply = chosen.clone();
                web.evaluate_js("window.chosen || ''", move |value| {
                    *reply.borrow_mut() = Some(value)
                });
                sleep(25).await;
                if matches!(&*chosen.borrow(), Some(Ok(value)) if value != "\"\"") {
                    break;
                }
            }
            let chosen = chosen.borrow().clone();
            check(
                "a file input asks the window's file dialog",
                files.asked.get() == Some(true)
                    && chosen == Some(Ok("\"zgx-fixture-upload.txt\"".into())),
                format!("multiple {:?}, chosen {chosen:?}", files.asked.get()),
            );
            // Printing: the app's `print`, then the page's `window.print()`,
            // each the print panel as a sheet on the window, cancelled here.
            let mut printed = Vec::new();
            for from_page in [false, true] {
                if from_page {
                    web.run_js("window.print()");
                } else {
                    web.print();
                }
                let mut sheet = None;
                for _ in 0..80 {
                    sleep(50).await;
                    sheet = attached_sheet();
                    if sheet.is_some() {
                        break;
                    }
                }
                // A sheet holding a print panel's views.
                printed.push(sheet.as_ref().is_some_and(|(_, sheet)| {
                    sheet
                        .contentView()
                        .is_some_and(|view| holds_print_view(&view))
                }));
                if let Some((parent, sheet)) = sheet {
                    // NSModalResponseCancel.
                    parent.endSheet_returnCode(&sheet, 0);
                }
                sleep(500).await;
            }
            check(
                "print and the page's window.print() show the print panel on the window",
                printed == [true, true],
                format!("{printed:?}"),
            );
            // Find: the heading's word, found and selected; a word the page
            // hasn't, not; then cleared.
            let finds = Rc::new(RefCell::new(Vec::new()));
            for text in ["Hello", "nowhere-in-the-page"] {
                let finds = finds.clone();
                web.find(text, Default::default(), move |found| {
                    finds.borrow_mut().push(found.found)
                });
            }
            for _ in 0..40 {
                if finds.borrow().len() == 2 {
                    break;
                }
                sleep(25).await;
            }
            web.clear_find();
            let finds = finds.borrow().clone();
            check(
                "find finds the heading's word and not a word the page hasn't",
                finds == [true, false],
                format!("{finds:?}"),
            );
            // Downloads: the app's handler hears each, with its name and
            // type. A `download` link it saves (here, in a temporary folder);
            // a response the page can't show (an iframe's octet stream) it
            // cancels, which saves nothing. It decides by name: WebKit names
            // and types the octet stream as its version sniffs it
            // (`application/octet-stream`, or `text/plain` as `Unknown.txt`).
            let folder = std::env::temp_dir().join(format!("zgx-fixture-downloads-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&folder);
            let asked = Rc::new(RefCell::new(Vec::new()));
            let statuses = Rc::new(RefCell::new(Vec::new()));
            {
                let (asked, statuses, folder) = (asked.clone(), statuses.clone(), folder.clone());
                web.set_download_handler(move |request, responder| {
                    asked.borrow_mut().push((request.name.clone(), request.mime.clone()));
                    if request.name == "zgx-fixture.txt" {
                        let statuses = statuses.clone();
                        responder.save_to(folder.join(&request.name), move |status| {
                            statuses.borrow_mut().push(status)
                        });
                    }
                });
            }
            web.run_js(
                "const a = document.createElement('a'); a.href = 'data:text/plain,saved by the app'; \
                 a.download = 'zgx-fixture.txt'; document.body.appendChild(a); a.click();",
            );
            for _ in 0..80 {
                sleep(50).await;
                if statuses.borrow().contains(&zgx::web_view::DownloadStatus::Finished) {
                    break;
                }
            }
            web.run_js(
                "const f = document.createElement('iframe'); f.style.display = 'none'; \
                 f.src = 'data:application/octet-stream,not shown'; document.body.appendChild(f);",
            );
            for _ in 0..80 {
                sleep(50).await;
                if asked.borrow().len() == 2 {
                    break;
                }
            }
            sleep(300).await;
            let saved = std::fs::read_to_string(folder.join("zgx-fixture.txt")).ok();
            let others = std::fs::read_dir(&folder).map_or(0, |entries| entries.count());
            let _ = std::fs::remove_dir_all(&folder);
            check(
                "a download is the app's: saved where it says, or nothing",
                asked.borrow().first() == Some(&("zgx-fixture.txt".into(), "text/plain".into()))
                    && asked.borrow().get(1).is_some_and(|(name, _)| name != "zgx-fixture.txt")
                    && saved.as_deref() == Some("saved by the app")
                    && others == 1
                    && statuses.borrow().last() == Some(&zgx::web_view::DownloadStatus::Finished),
                format!("asked {:?}, saved {saved:?}, files {others}, statuses {:?}", asked.borrow(), statuses.borrow()),
            );
            web.run_js("window.zgx.postMessage({hello: 1})");
            sleep(200).await;
            check(
                "page messages",
                messages.with_untracked(|count| *count) >= 1,
                format!("{} messages", messages.with_untracked(|count| *count)),
            );
            // The composer: overlay glass over the page.
            let composer = bounds("composer");
            let backdrops = diagnostics.backdrops();
            check(
                "composer backdrop shaped to the node",
                backdrops.iter().any(|b| {
                    close(b.frame, composer)
                        && !b.hidden
                        && (b.corner_radius - 22.).abs() < 0.01
                        && b.blur.is_some_and(|blur| (blur - 22.).abs() < 0.01)
                }),
                format!("{backdrops:?} vs {composer:?}"),
            );
            let overlay = diagnostics.overlay();
            check(
                "overlay plane covers the composer, above the page",
                overlay.is_some_and(|(rect, index)| {
                    rect.x <= composer.x
                        && rect.y <= composer.y
                        && rect.x + rect.width >= composer.x + composer.width
                        && rect.y + rect.height >= composer.y + composer.height
                        && report.as_ref().is_some_and(|page| index > page.index)
                }),
                format!("{overlay:?}"),
            );
            check(
                "backdrops between the page and the overlay",
                backdrops.iter().all(|b| {
                    report.as_ref().is_some_and(|page| b.index > page.index)
                        && overlay.is_some_and(|(_, index)| b.index < index)
                }) && backdrops.iter().all(|b| b.class == "CABackdropLayer"),
                format!("{backdrops:?}"),
            );
            let center = |r: zgx::scene::Rect| (r.x + r.width / 2., r.y + r.height / 2.);
            let page_rect = bounds("page");
            let (px, py) = (page_rect.x + page_rect.width / 2., page_rect.y + 160.);
            check(
                "clicks on the page reach WebKit",
                diagnostics.native_hit(px, py) == Some(page),
                format!("{:?}", diagnostics.native_hit(px, py)),
            );
            let (cx_, cy_) = center(composer);
            check(
                "clicks on the composer reach zgx",
                diagnostics.native_hit(cx_, cy_).is_none(),
                format!("{:?}", diagnostics.native_hit(cx_, cy_)),
            );
            // Open the frosted menu over the page.
            menu_open.set(true);
            sleep(300).await;
            let menu = bounds("menu");
            let backdrops = diagnostics.backdrops();
            check(
                "menu backdrop",
                // Exactly the rounded panel: no square backdrop at its corners.
                backdrops.iter().any(|b| {
                    close(b.frame, menu) && !b.hidden && (b.corner_radius - 14.).abs() < 0.01
                }),
                format!("{backdrops:?} vs {menu:?}"),
            );
            let (mx, my) = center(menu);
            check(
                "clicks on the menu reach zgx",
                diagnostics.native_hit(mx, my).is_none(),
                format!("{:?}", diagnostics.native_hit(mx, my)),
            );
            // The popover's backdrop listens for dismissal: the page is
            // covered for input while it is open.
            check(
                "menu open: page clicks dismiss it in zgx",
                diagnostics.native_hit(px, py).is_none(),
                format!("{:?}", diagnostics.native_hit(px, py)),
            );
            if let Some(number) = diagnostics.window_number() {
                println!("WINDOW {number}");
            }
            println!("SCREENSHOT menu-open");
            sleep(2500).await;
            menu_open.set(false);
            sleep(300).await;
            check(
                "menu closed: page clicks reach WebKit",
                diagnostics.native_hit(px, py) == Some(page),
                format!("{:?}", diagnostics.native_hit(px, py)),
            );
            check(
                "menu backdrop gone",
                diagnostics.backdrops().len() == 1,
                format!("{:?}", diagnostics.backdrops()),
            );
            println!("SCREENSHOT menu-closed");
            sleep(1500).await;
            // Keyboard focus: the page's follows the node's, both ways.
            let input = ui_input.clone();
            input.focus(&scene, Some(page));
            sleep(150).await;
            check(
                "zgx focus gives the page keyboard focus",
                web.focused(),
                format!("focused={}", web.focused()),
            );
            // The pointer over the page. While the page has keyboard focus
            // AppKit sends it the moves, and the pointer monitor routes them
            // as the host routes its own: these are mouse-moved NSEvents made
            // for this window and handed to the monitor, never sent.
            input.focus(&scene, Some(page));
            sleep(150).await;
            // What follows `web.pointer` (a script's edge affordance): a
            // page nothing follows is told nothing.
            let follow = runtime.effect({
                let web = web.clone();
                move || {
                    let _ = web.pointer();
                }
            });
            let page_rect = bounds("page");
            let menu_button = node("menu-button");
            let (bx, by) = center(bounds("menu-button"));
            diagnostics.pointer_moved(bx, by);
            sleep(100).await;
            check(
                "pointer over zgx: zgx hovers",
                input.hovered() == Some(menu_button) && diagnostics.pointer_over().is_none(),
                format!("{:?} {:?}", input.hovered(), diagnostics.pointer_over()),
            );
            diagnostics.pointer_moved(px, py);
            sleep(100).await;
            check(
                "pointer onto the page: zgx hears a leave",
                input.hovered().is_none() && diagnostics.pointer_over() == Some(page),
                format!("{:?} {:?}", input.hovered(), diagnostics.pointer_over()),
            );
            check(
                "the page hears the pointer",
                at(web.pointer(), (px - page_rect.x, py - page_rect.y)),
                format!("{:?}", web.pointer()),
            );
            diagnostics.pointer_moved(px + 30., py + 10.);
            sleep(100).await;
            check(
                "moves over the page: the page follows, zgx hovers nothing",
                at(
                    web.pointer(),
                    (px + 30. - page_rect.x, py + 10. - page_rect.y),
                ) && input.hovered().is_none(),
                format!("{:?} {:?}", web.pointer(), input.hovered()),
            );
            let (cx_, cy_) = center(bounds("composer"));
            diagnostics.pointer_moved(cx_, cy_);
            sleep(100).await;
            check(
                "over zgx drawn on the page: zgx's again, the page still knows",
                diagnostics.pointer_over().is_none() && web.pointer().is_some(),
                format!("{:?} {:?}", diagnostics.pointer_over(), web.pointer()),
            );
            diagnostics.pointer_moved(bx, by);
            sleep(100).await;
            check(
                "back over the toolbar: hovered, and the page heard it go",
                input.hovered() == Some(menu_button) && web.pointer().is_none(),
                format!("{:?} {:?}", input.hovered(), web.pointer()),
            );
            drop(follow);
            // Nothing follows the pointer over the page now: moving over it
            // posts nothing, so the host is not woken for it.
            diagnostics.pointer_moved(px, py);
            sleep(100).await;
            let mut posted = 0;
            for step in 0..50 {
                diagnostics.pointer_moved(px + step as f32, py);
                posted += usize::from(zgx::native_view::deliver_posted());
            }
            check(
                "moves over a page nothing follows wake nothing",
                posted == 0 && web.pointer().is_none(),
                format!("{posted} of 50 moves posted"),
            );
            // Idle: what the window costs while nothing happens.
            let cpu = || {
                std::process::Command::new("ps")
                    .args(["-o", "time=", "-p", &std::process::id().to_string()])
                    .output()
                    .ok()
                    .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            };
            let before = cpu();
            sleep(3000).await;
            println!(
                "IDLE cpu time before {before:?}, after 3 s idle {:?}",
                cpu()
            );
            input.focus(&scene, None);
            sleep(150).await;
            check(
                "zgx blur takes it back",
                !web.focused(),
                format!("focused={}", web.focused()),
            );
            // A window resize: the page follows layout in the same frames.
            window.set_inner_size(900., 640.);
            sleep(500).await;
            let natives = diagnostics.natives();
            let resized = natives.iter().find(|r| r.node == page).cloned();
            check(
                "resize: page frame and cut follow layout",
                resized.as_ref().is_some_and(|r| {
                    framed(r.frame, bounds("page")) && cut_to(&r.masks[..], bounds("page"), 14.)
                }),
                format!("{resized:?} vs {:?}", bounds("page")),
            );
            let composer = bounds("composer");
            check(
                "resize: composer backdrop follows",
                diagnostics
                    .backdrops()
                    .iter()
                    .any(|b| close(b.frame, composer)),
                format!("{:?} vs {composer:?}", diagnostics.backdrops()),
            );
            println!("SCREENSHOT resized");
            sleep(1200).await;
            window.set_inner_size(1100., 760.);
            sleep(300).await;
            // Hidden with its node, shown again where it is.
            page_shown.set(false);
            sleep(200).await;
            let hidden = diagnostics
                .natives()
                .iter()
                .find(|r| r.node == page)
                .map(|r| r.hidden);
            check(
                "hidden node hides the page",
                hidden == Some(true),
                format!("{hidden:?}"),
            );
            check(
                "hidden page takes no clicks",
                diagnostics.native_hit(px, py).is_none(),
                format!("{:?}", diagnostics.native_hit(px, py)),
            );
            page_shown.set(true);
            sleep(200).await;
            let shown = diagnostics
                .natives()
                .iter()
                .find(|r| r.node == page)
                .cloned();
            check(
                "shown again in place",
                shown
                    .as_ref()
                    .is_some_and(|r| !r.hidden && framed(r.frame, bounds("page"))),
                format!("{shown:?}"),
            );
            // Full screen: the page's own request (WebKit's window over
            // the screen), seen in its state; then exited, the page back in
            // its node.
            web.run_js("document.querySelector('.stripes').requestFullscreen()");
            let mut entered = false;
            for _ in 0..80 {
                sleep(50).await;
                if web.state().with_untracked(|state| state.full_screen) {
                    entered = true;
                    break;
                }
            }
            check(
                "a page's full screen shows in its state",
                entered,
                format!("full_screen={entered}"),
            );
            sleep(1500).await;
            web.exit_full_screen();
            let mut left = false;
            for _ in 0..80 {
                sleep(50).await;
                if !web.state().with_untracked(|state| state.full_screen) {
                    left = true;
                    break;
                }
            }
            sleep(1500).await;
            let back = diagnostics
                .natives()
                .iter()
                .find(|r| r.node == page)
                .cloned();
            check(
                "exiting full screen puts the page back in its node",
                left && back
                    .as_ref()
                    .is_some_and(|r| !r.hidden && framed(r.frame, bounds("page"))),
                format!("left={left} {back:?}"),
            );
            println!("FIXTURE {} failures", failures);
            println!("LAYERS\n{}", diagnostics.layer_tree());
            window.close();
            if failures > 0 {
                std::process::exit(1);
            }
        });
    }
    // The fixture checks AppKit state; elsewhere there is nothing to check.
    #[cfg(not(target_os = "macos"))]
    let _ = (
        cx, web, view, menu_open, page_shown, finished, messages, files,
    );
}
