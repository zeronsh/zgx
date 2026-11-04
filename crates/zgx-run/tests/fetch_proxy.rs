//! A proxy in the environment carries the app's requests, as ever, but never
//! a plugin's: a proxy would connect for it, past the app's address checks.
//! A process of its own: the environment is read as the first agent is made.
#![cfg(feature = "fetch")]
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use zgx_script::{Capability, Plugin, ScriptHost, testing::TestApp};

/// A server answering each request `body`; how many reached it.
fn serve(body: &'static str) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = hits.clone();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            counted.fetch_add(1, Ordering::SeqCst);
            let mut asked = Vec::new();
            let mut buffer = [0; 4096];
            while !asked.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buffer) {
                    Ok(n) if n > 0 => asked.extend_from_slice(&buffer[..n]),
                    _ => break,
                }
            }
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (port, hits)
}

fn script(url: &str) -> String {
    format!(
        r#"
local out = ui.state("out", "")
ui.fetch("{url}"):after(
    function(r) out:set(r.body) end,
    function(e) out:set(`failed {{e}}`) end
)
local shown = ui.text {{ id = "out", function() return out:get() end }}
"#
    )
}

#[test]
fn a_plugin_ignores_the_proxy_the_app_uses() {
    let (proxy, through_proxy) = serve("proxied");
    let (direct, reached) = serve("direct");
    // SAFETY: before any thread reads the environment; this binary's only test.
    unsafe {
        for name in [
            "NO_PROXY",
            "no_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ] {
            std::env::remove_var(name);
        }
        std::env::set_var("HTTP_PROXY", format!("http://127.0.0.1:{proxy}"));
        std::env::set_var("http_proxy", format!("http://127.0.0.1:{proxy}"));
    }
    let url = format!("http://127.0.0.1:{direct}/");

    // A plugin's request goes straight to the server.
    let dir = std::env::temp_dir().join(format!("zgx-run-proxy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(
        &entry,
        format!("{}\nui.contribute(\"bar\", shown)", script(&url)),
    )
    .unwrap();
    let mut plugin = TestApp::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", r#"return ui.column { ui.slot("bar") }"#);
        zgx_run::install_fetcher(&host);
        host.set_plugins(vec![Plugin {
            name: "plug".into(),
            entry,
            grants: vec![Capability::Fetch],
        }]);
        host
    });
    assert!(
        plugin.wait_until(|app| !app.text("out").is_empty()),
        "{:?}",
        plugin.error()
    );
    assert_eq!(plugin.text("out"), "direct");
    assert_eq!(reached.load(Ordering::SeqCst), 1);
    assert_eq!(
        through_proxy.load(Ordering::SeqCst),
        0,
        "a plugin's request took the proxy"
    );

    // The app's goes through the proxy, as it always has.
    let code = format!("{}\nreturn ui.column {{ shown }}", script(&url));
    let mut app = TestApp::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", code);
        zgx_run::install_fetcher(&host);
        host
    });
    assert!(
        app.wait_until(|app| !app.text("out").is_empty()),
        "{:?}",
        app.error()
    );
    // (This proxy tunnels nothing: reaching it is the point.)
    assert_eq!(through_proxy.load(Ordering::SeqCst), 1);
    assert_eq!(
        reached.load(Ordering::SeqCst),
        1,
        "the app's request went round the proxy"
    );
}
