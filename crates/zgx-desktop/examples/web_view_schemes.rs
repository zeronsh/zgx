//! A page served from a scheme of the app's own, with a script injected
//! before the page's and a navigation policy (WKWebView on macOS, WebView2
//! on Windows, WebKitGTK on Linux).
//!
//! `cargo run -p zgx-desktop --example web_view_schemes`
//! `-- --fixture`: checks each against the live page, prints `FIXTURE`
//! lines and exits (non-zero on failure). It also serves two sites of its
//! own over HTTP (`127.0.0.1` and `localhost`, one port), each setting a
//! cookie and local storage, and forgets the first (`clear_site_data`):
//! its data goes, the other's stays.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};
use zgx::{
    compose::prelude::*,
    web_view::{
        SchemeHandler, SchemeResponse, UserScript, WebView, WebViewEvent, WebViewOptions, web_view,
    },
};
use zgx_desktop::{Application, WindowOptions};

const PAGE: &str = r#"<!doctype html><meta charset="utf-8"><title>served</title>
<body style="font: 20px -apple-system, sans-serif; padding: 40px">
<h1>Served by the app</h1>
<p id="facts"></p>
<script>
  document.getElementById("facts").textContent = JSON.stringify({
    origin: location.origin, secure: isSecureContext, injected: window.injectedFirst === true,
  })
  window.zgx.postMessage({ hello: "from the page" })
</script>"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = std::env::args().any(|arg| arg == "--fixture");
    Application::new()
        .window(WindowOptions {
            title: "zgx web view schemes".into(),
            width: 900.,
            height: 600.,
            ..Default::default()
        })
        .run(move |cx| {
            // Served from a thread, as a real handler (disk, network) would.
            let demo = SchemeHandler::new("demo", |request, responder| {
                std::thread::spawn(move || {
                    let path = request
                        .url
                        .split_once("://")
                        .map(|(_, rest)| rest)
                        .unwrap_or("");
                    let response = match path {
                        p if p.ends_with("/missing") => return drop(responder),
                        _ => SchemeResponse {
                            status: 200,
                            headers: vec![(
                                "content-type".into(),
                                "text/html; charset=utf-8".into(),
                            )],
                            body: PAGE.as_bytes().to_vec(),
                        },
                    };
                    responder.respond(response);
                });
            });
            let web = WebView::new(
                &cx.ui.runtime,
                WebViewOptions {
                    // `https` is the web view's own: left out, with a warning, not a crash.
                    schemes: vec![demo, SchemeHandler::new("https", |_, _| {})],
                    scripts: vec![UserScript {
                        source: "window.injectedFirst = true".into(),
                        main_frame_only: true,
                    }],
                    ..Default::default()
                },
            );
            // Whether the policy was asked about the blocked page (the fixture's).
            let refused = Rc::new(Cell::new(false));
            let asked = refused.clone();
            web.set_navigation_policy(move |request| {
                let blocked = request.url.contains("blocked");
                asked.set(asked.get() || blocked);
                !blocked
            });
            web.load_url("demo://site.one/");
            if fixture {
                run_fixture(cx, web.clone(), refused);
            }
            cx.render(column().grow().child(web_view(&web).grow()));
        })
}

/// Serves `/set` (a cookie, `Set-Cookie`, and local storage, set by its
/// script) and anything else as a page that sets nothing, on one port of
/// 127.0.0.1 and, where it can, ::1 (`localhost` may resolve to either).
fn serve_sites() -> u16 {
    use std::io::{BufRead, BufReader, Write};
    let v4 = std::net::TcpListener::bind("127.0.0.1:0").expect("a local port");
    let port = v4.local_addr().unwrap().port();
    let v6 = std::net::TcpListener::bind(("::1", port)).ok();
    for listener in std::iter::once(v4).chain(v6) {
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                let mut reader = BufReader::new(&stream);
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                // The headers, to the blank line.
                let mut header = String::new();
                while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
                    header.clear();
                }
                let set = line.split(' ').nth(1).is_some_and(|path| path == "/set");
                let (cookie, body) = match set {
                    true => (
                        "Set-Cookie: probe=1; Path=/; Max-Age=3600\r\n",
                        "<title>set</title><script>localStorage.setItem('probe', '1')</script>set",
                    ),
                    false => ("", "<title>read</title>read"),
                };
                let mut stream = &stream;
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n{cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
    }
    port
}

/// What `js` evaluates to on `web`'s page (its JSON), or an empty string
/// if it gave nothing in 5 seconds.
async fn evaluate(web: &WebView, js: &str) -> String {
    let reply = Rc::new(RefCell::new(None));
    let answer = reply.clone();
    web.evaluate_js(js, move |result| *answer.borrow_mut() = Some(result));
    within(5, || reply.borrow().is_some()).await;
    reply
        .borrow_mut()
        .take()
        .and_then(Result::ok)
        .unwrap_or_default()
}

/// What `web.clear_site_data(url)` answered, within 10 seconds.
async fn clear_site(web: &WebView, url: &str) -> Option<Result<usize, String>> {
    let reply = Rc::new(RefCell::new(None));
    let answer = reply.clone();
    web.clear_site_data(url, move |result| *answer.borrow_mut() = Some(result));
    within(10, || reply.borrow().is_some()).await;
    reply.borrow_mut().take()
}

/// Whether `done` came within `seconds`: each check waits for what it reads,
/// not a fixed time (two seconds once passed on CI with nothing loaded, and
/// every check failed). All together they stay under the probe's minute.
async fn within(seconds: u64, done: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !done() {
        if Instant::now() > deadline {
            return false;
        }
        zgx::timer::sleep(Duration::from_millis(50)).await;
    }
    true
}

fn run_fixture(cx: &mut zgx_desktop::WindowContext, web: WebView, refused: Rc<Cell<bool>>) {
    let failures = Rc::new(RefCell::new(0_u32));
    let check = {
        let failures = failures.clone();
        move |ok: bool, what: &str, detail: String| {
            println!(
                "FIXTURE {} {what}: {detail}",
                if ok { "ok" } else { "FAILED" }
            );
            if !ok {
                *failures.borrow_mut() += 1;
            }
        }
    };
    let failed_loads = Rc::new(RefCell::new(Vec::<String>::new()));
    let senders = Rc::new(RefCell::new(Vec::<(String, bool)>::new()));
    let finished = Rc::new(RefCell::new(Vec::<String>::new()));
    // Every event and when it came, printed if a check fails.
    let events = Rc::new(RefCell::new(Vec::<String>::new()));
    let started = Instant::now();
    let subscription = {
        let (failed_loads, senders, finished, events) = (
            failed_loads.clone(),
            senders.clone(),
            finished.clone(),
            events.clone(),
        );
        web.on_event(move |event| {
            events
                .borrow_mut()
                .push(format!("{:.2}s {event:?}", started.elapsed().as_secs_f64()));
            match event {
                WebViewEvent::NavigationFinished { url } => finished.borrow_mut().push(url.clone()),
                WebViewEvent::NavigationFailed { url, .. } => {
                    failed_loads.borrow_mut().push(url.clone());
                }
                WebViewEvent::Message {
                    origin, main_frame, ..
                } => senders.borrow_mut().push((origin.clone(), *main_frame)),
                _ => {}
            }
        })
    };
    cx.tasks.spawn(async move {
        let _subscription = subscription;
        within(20, || {
            finished.borrow().iter().any(|url| url.contains("site.one"))
        })
        .await;
        let reply = Rc::new(RefCell::new(None));
        let answer = reply.clone();
        web.evaluate_js(
            "document.getElementById('facts').textContent",
            move |result| *answer.borrow_mut() = Some(result),
        );
        within(5, || reply.borrow().is_some()).await;
        let facts = reply
            .borrow_mut()
            .take()
            .and_then(Result::ok)
            .unwrap_or_default();
        check(
            facts.contains(r#"\"origin\":\"demo://site.one\""#),
            "own origin",
            facts.clone(),
        );
        check(
            facts.contains(r#"\"secure\":true"#),
            "secure context",
            facts.clone(),
        );
        check(
            facts.contains(r#"\"injected\":true"#),
            "script ran before the page's",
            facts.clone(),
        );
        within(5, || !senders.borrow().is_empty()).await;
        check(
            senders
                .borrow()
                .first()
                .is_some_and(|(origin, top)| origin == "demo://site.one" && *top),
            "a message says who sent it",
            format!("{:?}", senders.borrow()),
        );

        web.load_url("demo://blocked.example/");
        let asked = within(5, || refused.get()).await;
        check(
            asked && within(5, || web.url().contains("site.one")).await,
            "policy cancels a navigation",
            web.url(),
        );

        web.load_url("demo://site.one/missing");
        let failed = within(10, || {
            failed_loads
                .borrow()
                .iter()
                .any(|url| url.ends_with("/missing"))
        })
        .await;
        check(
            failed,
            "a dropped responder fails the load",
            format!("{:?}", failed_loads.borrow()),
        );

        // A site forgotten: its cookie and storage go, another site's stay.
        let port = serve_sites();
        let (one, other) = (
            format!("http://127.0.0.1:{port}"),
            format!("http://localhost:{port}"),
        );
        let loaded = |url: String| {
            let finished = finished.clone();
            async move { within(20, || finished.borrow().iter().any(|u| u.starts_with(&url))).await }
        };
        let held = "document.cookie + '|' + localStorage.getItem('probe')";
        for site in [&one, &other] {
            web.load_url(format!("{site}/set"));
            loaded(format!("{site}/set")).await;
        }
        let cleared = clear_site(&web, &format!("{one}/any/page")).await;
        check(
            matches!(cleared, Some(Ok(n)) if n >= 1),
            "clear_site_data removes the site's records",
            format!("{cleared:?}"),
        );
        web.load_url(format!("{one}/read"));
        loaded(format!("{one}/read")).await;
        let left = evaluate(&web, held).await;
        check(left == "\"|null\"", "the forgotten site's cookie and storage are gone", left);
        web.load_url(format!("{other}/read"));
        loaded(format!("{other}/read")).await;
        let kept = evaluate(&web, held).await;
        check(kept == "\"probe=1|1\"", "another site's stay", kept);
        // Leave nothing behind in the profile.
        clear_site(&web, &other).await;

        let failures = *failures.borrow();
        if failures > 0 {
            println!("FIXTURE events: {:?}", events.borrow());
            println!(
                "FIXTURE state: {:?}",
                web.state().with_untracked(Clone::clone)
            );
        }
        println!("FIXTURE {failures} failures");
        std::process::exit(if failures == 0 { 0 } else { 1 });
    });
}
