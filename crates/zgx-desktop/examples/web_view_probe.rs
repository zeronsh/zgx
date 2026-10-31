//! A web page under test (Linux, `--features webkitgtk`; run by
//! `scripts/web_view_smoke.py`): a page in a box rounded by 24 points at
//! (40, 40), 600 × 400, with a red zgx square drawn over its bottom right
//! corner, and a zgx button before the page at (660, 40); on the page a
//! field at (20, 20), 300 × 40, a `<select>` at
//! (360, 20), 200 × 30, a green target at (20, 100), 200 × 80 (a pointer
//! cursor), a box with an image cursor (16 × 16 red, its hot spot at 4, 4)
//! at (360, 100), 120 × 60, a link (to https://example.com/dragged) at (20,
//! 240), a 60 × 60 red image at (360, 180), and enough height to scroll.
//! Beside the page, a gray zgx drop target at (660, 100), 90 × 60, prints
//! `DROPPED <text>` for each drag dropped on it.
//!
//! Prints `FINISHED <url>` as the page loads, `EVAL <json>` for a script it
//! then evaluates, `PAGE <json>` for each message the page sends (what it
//! heard: clicks, typing, compositions, keys, choices, scrolling) and `ACTION <name>`
//! for each of the app's shortcuts that fired: Ctrl+J (the page takes it),
//! Ctrl+K (the page leaves it alone) and Ctrl+L (reserved: the page never
//! hears it). Ctrl+I (reserved) opens Web Inspector; `--large-page` makes the
//! page's box 900 × 650.
//!
//! `--media URL`: the page at URL instead (the smoke's video and audio).
use zgx::{
    actions::{Action, KeyBinding, Keymap, Keystroke},
    compose::prelude::*,
    input::{EventPhase, InputEvent, Key, Modifiers},
    web_view::{WebView, WebViewEvent, WebViewOptions, web_view},
};
use zgx_desktop::{Application, MenuAction, WindowOptions};

const PAGE: &str = r#"<!doctype html><html><head><title>probe page</title><style>
html, body { margin: 0; background: rgb(0, 0, 255); height: 3000px; }
#field { position: absolute; left: 20px; top: 20px; width: 300px; height: 40px; font-size: 20px; box-sizing: border-box; }
#choice { position: absolute; left: 360px; top: 20px; width: 200px; height: 30px; }
#heading { position: absolute; left: 20px; top: 200px; margin: 0; font-size: 20px; color: white; }
#link { position: absolute; left: 20px; top: 240px; font-size: 20px; color: white; }
#pic { position: absolute; left: 360px; top: 180px; width: 60px; height: 60px; }
#target { position: absolute; left: 20px; top: 100px; width: 200px; height: 80px; background: rgb(0, 255, 0); cursor: pointer; }
#imaged { position: absolute; left: 360px; top: 100px; width: 120px; height: 60px; cursor: url(data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAGUlEQVR42mP4z8DwnxLMMGrAqAGjBgwXAwAwxP4QisZM5QAAAABJRU5ErkJggg==) 4 4, auto; }
</style></head><body><h1 id="heading">Probe heading</h1><input id="field" aria-label="Probe field"><select id="choice"><option>Alpha</option><option>Beta</option><option>Gamma</option></select><div id="target"></div><div id="imaged"></div><a id="link" href="https://example.com/dragged">Drag me</a><img id="pic" alt="red" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAGUlEQVR42mP4z8DwnxLMMGrAqAGjBgwXAwAwxP4QisZM5QAAAABJRU5ErkJggg=="><script>
const post = (value) => window.zgx.postMessage(value);
const field = document.getElementById('field');
document.getElementById('target').addEventListener('click', (e) => post({ click: [e.offsetX, e.offsetY], detail: e.detail }));
field.addEventListener('input', () => post({ value: field.value }));
field.addEventListener('focus', () => post({ focused: 'field' }));
const choice = document.getElementById('choice');
choice.addEventListener('change', () => post({ chose: choice.value }));
field.addEventListener('compositionstart', () => post({ composition: 'start' }));
field.addEventListener('compositionend', (e) => post({ composition: 'end', data: e.data }));
document.addEventListener('keydown', (e) => {
  post({ key: e.key, code: e.code, ctrl: e.ctrlKey });
  if (e.ctrlKey && e.key === 'j') e.preventDefault();
}, true);
addEventListener('resize', () => post({ resized: innerHeight }));
addEventListener('dragend', () => post({ dragend: true }));
let scrolled = false;
addEventListener('scroll', () => { if (!scrolled && scrollY > 0) { scrolled = true; post({ scrolled: scrollY }); } });
</script></body></html>"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `--large-page`: a page box of 900 × 650 (in a window to fit), large
    // enough for WebKit to dock Web Inspector in it.
    let large = std::env::args().any(|arg| arg == "--large-page");
    let (page_width, page_height): (f32, f32) = if large { (900., 650.) } else { (600., 400.) };
    Application::new()
        .window(WindowOptions {
            title: "Web view probe".into(),
            width: f64::from(page_width) + 160.,
            height: f64::from(page_height) + 120.,
            ..Default::default()
        })
        .run(|cx| {
            // `--transparent`: a page with no background of its own, over
            // its box's yellow.
            let transparent = std::env::args().any(|arg| arg == "--transparent");
            let options = WebViewOptions {
                transparent,
                inspectable: true,
                ..Default::default()
            };
            let web = WebView::new(&cx.ui.runtime, options);
            let mut args = std::env::args();
            if let Some(url) = args
                .by_ref()
                .position(|arg| arg == "--media")
                .and(args.next())
            {
                web.load_url(url);
            } else {
                let page = if transparent {
                    PAGE.replace("background: rgb(0, 0, 255)", "background: transparent")
                } else {
                    PAGE.into()
                };
                web.load_html(page, Some("https://probe.zgx.test/"));
            }
            let evaluate = web.clone();
            let events = web.on_event(move |event| match event {
                WebViewEvent::NavigationFinished { url } => {
                    println!("FINISHED {url}");
                    evaluate.evaluate_js("document.title + ' ' + (1 + 2)", |result| match result {
                        Ok(json) => println!("EVAL {json}"),
                        Err(error) => println!("EVAL error {error}"),
                    });
                }
                WebViewEvent::Message {
                    body,
                    origin,
                    main_frame,
                } => {
                    println!("PAGE {body} origin={origin} main={main_frame}")
                }
                _ => {}
            });
            cx.render(
                div()
                    .w_full()
                    .h_full()
                    .bg(rgba(0x202020ff))
                    .child(
                        web_view(&web)
                            .absolute()
                            .left(40.)
                            .top(40.)
                            .w(page_width)
                            .h(page_height)
                            .rounded(24.)
                            .bg(rgba(0xffff00ff)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(560.)
                            .top(360.)
                            .w(120.)
                            .h(120.)
                            .bg(rgba(0xff0000ff)),
                    )
                    // A zgx drop target for drags out of the page.
                    .child(
                        div()
                            .absolute()
                            .left(660.)
                            .top(100.)
                            .w(90.)
                            .h(60.)
                            .bg(rgba(0x808080ff))
                            .on_drop(|text: &zgx::input::DraggedText, _| {
                                println!("DROPPED {}", text.text())
                            }),
                    )
                    // zgx content before the page in Tab's order.
                    .child(
                        button()
                            .id("before")
                            .absolute()
                            .left(660.)
                            .top(40.)
                            .w(90.)
                            .h(30.)
                            .accessible_label("Before page")
                            .child(text("Before page")),
                    ),
            );
            let root = cx.ui.root();
            let inspect = web.clone();
            cx.ui.on_event(root, false, move |event| {
                if event.phase != EventPhase::Capture
                    && let InputEvent::Action(action) = &event.event
                    && let Some(action) = action.downcast_ref::<MenuAction>()
                {
                    println!("ACTION {}", action.0);
                    if action.0 == "i" {
                        inspect.open_devtools();
                    }
                    event.prevent_default();
                }
            });
            let control = Modifiers {
                control: true,
                ..Default::default()
            };
            let bind = |key: &str| {
                KeyBinding::new(
                    [Keystroke::new(Key::Character(key.into()), control)],
                    Action::new(MenuAction(key.into())),
                )
                .unwrap()
            };
            let keymap = cx.ui.input.bind_keys(
                root,
                Keymap::new()
                    .bind(bind("j"))
                    .bind(bind("k"))
                    .bind(bind("l").reserved())
                    .bind(bind("i").reserved()),
            );
            cx.tasks.spawn(async move {
                let _keep = (events, keymap, web);
                std::future::pending::<()>().await;
            });
        })
}
