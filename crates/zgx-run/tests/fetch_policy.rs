//! The app's `FetchPolicy` over what a script asks of the network, against a
//! server on this machine: each request says who asked, every redirect's URL
//! is asked before it is followed, every address before it is connected to
//! (a resolved name's and an IP literal's), a plugin's headers are held to
//! what it may set, and remote images and `ui.socket` go through the same
//! hooks. The proxy in the environment is `fetch_proxy.rs` (a process of its
//! own).
#![cfg(all(feature = "fetch", feature = "socket"))]
use std::{
    io::{Read, Write},
    net::{IpAddr, TcpListener},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use zgx_script::{Capability, FetchPolicy, Plugin, ScriptHost, Url, testing::TestApp};

/// A server: `/hop` answers `status` to `location`, the rest 200 with the
/// `authorization` header it was sent. How many requests reached it.
struct Server {
    base: String,
    port: u16,
    hits: Arc<AtomicUsize>,
}
fn serve(status: u16, location: &str) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(AtomicUsize::new(0));
    let (counted, location) = (hits.clone(), location.to_owned());
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
            let head = String::from_utf8_lossy(&asked).into_owned();
            let first = head.lines().next().unwrap_or_default().to_owned();
            let path = first.split(' ').nth(1).unwrap_or_default();
            let method = first.split(' ').next().unwrap_or_default();
            let reply = if path == "/hop" {
                format!(
                    "HTTP/1.1 {status} Moved\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
            } else {
                let auth = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("authorization")
                            .then(|| value.trim().to_owned())
                    })
                    .unwrap_or_default();
                let body = format!("{method} auth={auth}");
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    Server {
        base: format!("http://127.0.0.1:{port}"),
        port,
        hits,
    }
}

/// A policy that records what it is asked, and refuses what it was told to.
#[derive(Clone, Default)]
struct Rules {
    asked: Arc<Mutex<Vec<String>>>,
    refuse_host: Option<&'static str>,
    refuse_addresses: bool,
    refuse_header: Option<&'static str>,
}
impl FetchPolicy for Rules {
    fn url(&self, caller: Option<&str>, url: &Url) -> Result<(), String> {
        self.asked
            .lock()
            .unwrap()
            .push(format!("url {caller:?} {}", url.host()));
        match self.refuse_host {
            Some(host) if url.host() == host => Err(format!("{host} is refused")),
            _ => Ok(()),
        }
    }
    fn address(&self, caller: Option<&str>, host: &str, address: IpAddr) -> Result<(), String> {
        self.asked.lock().unwrap().push(format!(
            "address {caller:?} {host} {}",
            address.is_loopback()
        ));
        if self.refuse_addresses {
            return Err(format!("{address} is refused"));
        }
        Ok(())
    }
    fn header(&self, caller: Option<&str>, host: &str, name: &str) -> Result<(), String> {
        self.asked
            .lock()
            .unwrap()
            .push(format!("header {caller:?} {host} {name}"));
        match self.refuse_header {
            Some(header) if name.eq_ignore_ascii_case(header) => Err(format!("{name} is refused")),
            _ => Ok(()),
        }
    }
}

/// `ui.fetch(url, opts)` as the script runs, its answer in the text `out`.
fn fetching(url: &str, opts: &str) -> String {
    format!(
        r#"
local out = ui.state("out", "")
ui.fetch("{url}", {opts}):after(
    function(r) out:set(`ok {{r.status}} {{r.body}}`) end,
    function(e) out:set(`failed {{e}}`) end
)
local shown = ui.text {{ id = "out", function() return out:get() end }}
"#
    )
}
/// `code` as a plugin's script, granted the network, under `rules`.
fn as_plugin(code: &str, rules: &Rules) -> TestApp {
    let dir = std::env::temp_dir().join(format!(
        "zgx-run-policy-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry: PathBuf = dir.join("init.luau");
    std::fs::write(&entry, format!("{code}\nui.contribute(\"bar\", shown)")).unwrap();
    let rules = rules.clone();
    TestApp::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", r#"return ui.column { ui.slot("bar") }"#);
        zgx_run::install_fetcher(&host);
        zgx_run::install_connector(&host);
        host.fetch_policy(rules);
        host.set_plugins(vec![Plugin {
            name: "plug".into(),
            entry,
            grants: vec![Capability::Fetch],
        }]);
        host
    })
}
/// `code` as the app's own script, under `rules`.
fn as_app(code: &str, rules: &Rules) -> TestApp {
    let code = format!("{code}\nreturn ui.column {{ shown }}");
    let rules = rules.clone();
    TestApp::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", code);
        zgx_run::install_fetcher(&host);
        host.fetch_policy(rules);
        host
    })
}
fn answered(app: &mut TestApp) -> String {
    assert!(
        app.wait_until(|app| !app.text("out").is_empty()),
        "no answer: {:?}",
        app.error()
    );
    app.text("out")
}

#[test]
fn each_request_says_who_asked() {
    let server = serve(200, "");
    let rules = Rules::default();
    let url = format!("{}/ok", server.base);
    let mut plugin = as_plugin(&fetching(&url, "{}"), &rules);
    assert!(answered(&mut plugin).starts_with("ok 200"));
    let mut app = as_app(&fetching(&url, "{}"), &rules);
    assert!(answered(&mut app).starts_with("ok 200"));
    let asked = rules.asked.lock().unwrap();
    assert!(
        asked.contains(&r#"url Some("plug") 127.0.0.1"#.to_owned()),
        "{asked:?}"
    );
    assert!(
        asked.contains(&"url None 127.0.0.1".to_owned()),
        "{asked:?}"
    );
    assert!(
        asked.contains(&r#"address Some("plug") 127.0.0.1 true"#.to_owned()),
        "{asked:?}"
    );
}

#[test]
fn a_redirect_to_a_refused_host_is_never_followed() {
    let target = serve(200, "");
    let first = serve(302, &format!("http://localhost:{}/ok", target.port));
    let rules = Rules {
        refuse_host: Some("localhost"),
        ..Default::default()
    };
    let mut app = as_plugin(&fetching(&format!("{}/hop", first.base), "{}"), &rules);
    let out = answered(&mut app);
    assert!(out.contains("localhost is refused"), "{out}");
    assert_eq!(first.hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        target.hits.load(Ordering::SeqCst),
        0,
        "the refused host was reached"
    );
}

#[test]
fn a_followed_redirect_goes_as_ureqs_own_did() {
    let target = serve(200, "");
    // Across hosts: no `Authorization` follows, a POST becomes a GET.
    let first = serve(302, &format!("{}/ok", target.base));
    let rules = Rules::default();
    let opts = r#"{ method = "POST", body = "x", headers = { Authorization = "secret" } }"#;
    let mut app = as_app(&fetching(&format!("{}/hop", first.base), opts), &rules);
    assert_eq!(answered(&mut app), "ok 200 GET auth=");
    // Each hop was asked of, its headers too.
    let hosts = rules.asked.lock().unwrap();
    assert_eq!(
        hosts.iter().filter(|a| a.starts_with("url None")).count(),
        2,
        "{hosts:?}"
    );

    // A relative location, on the same server.
    let relative = serve(301, "/ok");
    let mut app = as_app(
        &fetching(&format!("{}/hop", relative.base), "{}"),
        &Rules::default(),
    );
    assert_eq!(answered(&mut app), "ok 200 GET auth=");

    // A 307 of a request with a body isn't followed.
    let keeps = serve(307, "/ok");
    let opts = r#"{ method = "POST", body = "x" }"#;
    let mut app = as_app(
        &fetching(&format!("{}/hop", keeps.base), opts),
        &Rules::default(),
    );
    let out = answered(&mut app);
    assert!(out.starts_with("failed") && out.contains("307"), "{out}");

    // A redirect to itself ends, after the ten followed.
    let looping = serve(302, "/hop");
    let mut app = as_app(
        &fetching(&format!("{}/hop", looping.base), "{}"),
        &Rules::default(),
    );
    let out = answered(&mut app);
    assert!(out.contains("too many redirects"), "{out}");
    assert_eq!(looping.hits.load(Ordering::SeqCst), 11);
}

#[test]
fn a_name_resolving_to_a_refused_address_is_never_connected_to() {
    let server = serve(200, "");
    let rules = Rules {
        refuse_addresses: true,
        ..Default::default()
    };
    let url = format!("http://localhost:{}/ok", server.port);
    let mut app = as_plugin(&fetching(&url, "{}"), &rules);
    let out = answered(&mut app);
    assert!(out.contains("is refused"), "{out}");
    assert_eq!(
        server.hits.load(Ordering::SeqCst),
        0,
        "connected to a refused address"
    );
    let asked = rules.asked.lock().unwrap();
    assert!(
        asked
            .iter()
            .any(|a| a.starts_with(r#"address Some("plug") localhost"#)),
        "{asked:?}"
    );
}

#[test]
fn an_ip_literal_is_asked_of_too() {
    let server = serve(200, "");
    let rules = Rules {
        refuse_addresses: true,
        ..Default::default()
    };
    let mut app = as_plugin(&fetching(&format!("{}/ok", server.base), "{}"), &rules);
    let out = answered(&mut app);
    assert!(out.contains("is refused"), "{out}");
    assert_eq!(
        server.hits.load(Ordering::SeqCst),
        0,
        "connected to a refused literal"
    );
}

#[test]
fn a_plugin_never_sets_the_headers_that_steer_a_connection() {
    let server = serve(200, "");
    for name in [
        "Host",
        "connection",
        "Transfer-Encoding",
        "TE",
        "Upgrade",
        "Proxy-Authorization",
        "proxy-anything",
    ] {
        let opts = format!(r#"{{ headers = {{ ["{name}"] = "x" }} }}"#);
        let url = format!("{}/ok", server.base);
        let mut app = as_plugin(&fetching(&url, &opts), &Rules::default());
        let out = answered(&mut app);
        assert!(
            out.contains(&format!("can't set the {name} header")),
            "{name}: {out}"
        );
    }
    assert_eq!(server.hits.load(Ordering::SeqCst), 0);
    // The app's own scripts may, as before.
    let opts = r#"{ headers = { Host = "example.test" } }"#;
    let url = format!("{}/ok", server.base);
    let mut app = as_app(&fetching(&url, opts), &Rules::default());
    assert!(answered(&mut app).starts_with("ok 200"));
}

#[test]
fn the_policy_hears_a_plugins_other_headers() {
    let server = serve(200, "");
    let rules = Rules {
        refuse_header: Some("x-secret"),
        ..Default::default()
    };
    let url = format!("{}/ok", server.base);
    let opts = r#"{ headers = { ["x-fine"] = "1" } }"#;
    let mut app = as_plugin(&fetching(&url, opts), &rules);
    assert!(answered(&mut app).starts_with("ok 200"));
    assert!(
        rules
            .asked
            .lock()
            .unwrap()
            .contains(&r#"header Some("plug") 127.0.0.1 x-fine"#.to_owned())
    );
    let opts = r#"{ headers = { ["X-Secret"] = "1" } }"#;
    let mut app = as_plugin(&fetching(&url, opts), &rules);
    let out = answered(&mut app);
    assert!(out.contains("X-Secret is refused"), "{out}");
}

/// A script's remote image is a request of whoever shows it.
#[test]
fn a_plugins_remote_image_goes_through_the_policy() {
    let server = serve(200, "");
    let rules = Rules {
        refuse_addresses: true,
        ..Default::default()
    };
    let code = format!(
        r#"local shown = ui.image {{ id = "pic", src = "{}/pic.png", w = 10, h = 10 }}"#,
        server.base
    );
    let mut app = as_plugin(&code, &rules);
    assert!(app.wait_until(|_| {
        rules
            .asked
            .lock()
            .unwrap()
            .iter()
            .any(|a| a.starts_with(r#"address Some("plug")"#))
    }));
    assert_eq!(server.hits.load(Ordering::SeqCst), 0);
}

/// Refused to any plugin, whatever it asks for.
#[derive(Clone)]
struct NotForPlugins;
impl FetchPolicy for NotForPlugins {
    fn url(&self, caller: Option<&str>, _: &Url) -> Result<(), String> {
        match caller {
            Some(plugin) => Err(format!("not for {plugin}")),
            None => Ok(()),
        }
    }
}

/// A plugin's refused image is its own: the app showing the same URL loads
/// it at once, not after the plugin's backoff.
#[test]
fn a_plugins_refused_image_leaves_the_app_loading_it_at_once() {
    // A 2×2 PNG, however it is asked for.
    const PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 6,
        0, 0, 0, 114, 182, 13, 36, 0, 0, 0, 17, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240,
        31, 132, 25, 96, 12, 0, 71, 202, 7, 249, 103, 89, 110, 183, 0, 0, 0, 0, 73, 69, 78, 68,
        174, 66, 96, 130,
    ];
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
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                PNG.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(PNG);
        }
    });
    let url = format!("http://127.0.0.1:{port}/pic.png");
    let dir = std::env::temp_dir().join(format!("zgx-run-image-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(
        &entry,
        format!(
            r#"local why = ui.state("why", "")
ui.contribute("bar", ui.column {{
    ui.image {{ src = "{url}", w = 10, h = 10, on_error = function(e) why:set(e) end }},
    ui.text {{ id = "why", function() return why:get() end }},
}})"#
        ),
    )
    .unwrap();
    let code = format!(
        r#"local shown = ui.state("shown", false)
local loaded = ui.state("loaded", "")
return ui.column {{
    ui.button {{ id = "show", on_click = function() shown:set(true) end, "s" }},
    ui.slot("bar"),
    ui.show(shown, ui.image {{ src = "{url}", w = 10, h = 10,
        on_load = function(size) loaded:set(`{{size.w}}`) end }}),
    ui.text {{ id = "loaded", function() return loaded:get() end }},
}}"#
    );
    let mut app = TestApp::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", code);
        zgx_run::install_fetcher(&host);
        host.fetch_policy(NotForPlugins);
        host.set_plugins(vec![Plugin {
            name: "plug".into(),
            entry,
            grants: vec![Capability::Fetch],
        }]);
        host
    });
    assert!(
        app.wait_until(|app| !app.text("why").is_empty()),
        "{:?}",
        app.error()
    );
    assert!(
        app.text("why").contains("not for plug"),
        "{}",
        app.text("why")
    );
    assert_eq!(hits.load(Ordering::SeqCst), 0, "refused before connecting");

    let asked = std::time::Instant::now();
    app.click("show");
    assert!(
        app.wait_until(|app| app.text("loaded") == "2"),
        "{:?}",
        app.error()
    );
    // A failure's first backoff is five seconds.
    assert!(asked.elapsed().as_secs() < 4, "{:?}", asked.elapsed());
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

/// A socket's URL and addresses are asked of, as a plugin's.
#[test]
fn a_plugins_socket_goes_through_the_policy() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let socket = |host: &str| {
        format!(
            r#"
local socket = ui.socket("ws://{host}:{port}/", {{ reconnect = false }})
local shown = ui.text {{ id = "out", function()
    local e = socket.error
    return if e then `failed {{e}}` else ""
end }}
"#
        )
    };
    // Its URL.
    let rules = Rules {
        refuse_host: Some("localhost"),
        ..Default::default()
    };
    let mut app = as_plugin(&socket("localhost"), &rules);
    let out = answered(&mut app);
    assert!(out.contains("localhost is refused"), "{out}");
    assert!(
        rules
            .asked
            .lock()
            .unwrap()
            .contains(&r#"url Some("plug") localhost"#.to_owned())
    );
    // Its addresses, an IP literal's too.
    let rules = Rules {
        refuse_addresses: true,
        ..Default::default()
    };
    let mut app = as_plugin(&socket("127.0.0.1"), &rules);
    let out = answered(&mut app);
    assert!(out.contains("is refused"), "{out}");
    assert!(listener.accept().is_err(), "connected to a refused address");
}
