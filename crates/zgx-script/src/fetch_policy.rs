//! What an app lets its scripts reach of the network: the hooks a transport
//! calls as a script's request goes out (`ScriptHost::fetch_policy`). The
//! app owns the rules; zgx says who is asking, and what for.
use std::{cell::RefCell, net::IpAddr, rc::Rc, sync::Arc};

/// A URL a request goes to, as the transport that connects read it: its
/// `host` is the host it connects to, so a policy reading `host` can't
/// disagree with the transport over a URL parsed two ways.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Url {
    full: String,
    scheme: String,
    host: String,
    port: Option<u16>,
}
impl Url {
    /// A URL with these parts, for a transport. `host` has no brackets
    /// round an IPv6 address.
    pub fn new(
        full: impl Into<String>,
        scheme: impl Into<String>,
        host: impl Into<String>,
        port: Option<u16>,
    ) -> Self {
        Self {
            full: full.into(),
            scheme: scheme.into(),
            host: host.into(),
            port,
        }
    }
    /// The whole URL.
    pub fn as_str(&self) -> &str {
        &self.full
    }
    /// `https`, `http`, `wss`, `ws`.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }
    /// The host it connects to, a name or an IP address (no brackets).
    pub fn host(&self) -> &str {
        &self.host
    }
    /// The port, if the URL says one.
    pub fn port(&self) -> Option<u16> {
        self.port
    }
}

/// What an app allows of its scripts' requests (`ui.fetch`, remote images,
/// `ui.socket`). `caller` is the plugin asking (its name), `None` for the
/// app's own scripts. An `Err` refuses the request, its message the script's
/// error. Each hook allows everything unless the app says otherwise. They
/// run on the request's thread, not the window's.
///
/// `zgx-run`'s transports call them; a host bringing its own transport
/// reads the caller and this policy from `Body` and `Socket`, and calls them
/// itself.
pub trait FetchPolicy: Send + Sync + 'static {
    /// A URL about to be asked for: the first, and every redirect's.
    fn url(&self, caller: Option<&str>, url: &Url) -> Result<(), String> {
        let _ = (caller, url);
        Ok(())
    }
    /// An address `host` resolved to, about to be connected to; an IP
    /// literal's own address too. Every one of them is asked, before any is
    /// connected to, so a name answering with one address allowed and one
    /// refused is refused (DNS rebinding).
    fn address(&self, caller: Option<&str>, host: &str, address: IpAddr) -> Result<(), String> {
        let _ = (caller, host, address);
        Ok(())
    }
    /// A header the script set, about to go to `host`, on every hop. A
    /// plugin never sets `Host`, `Connection`, `Transfer-Encoding`, `TE`,
    /// `Upgrade` or `Proxy-*`, whatever this says.
    fn header(&self, caller: Option<&str>, host: &str, name: &str) -> Result<(), String> {
        let _ = (caller, host, name);
        Ok(())
    }
}

/// The app's policy, shared with what makes requests. None: all allowed.
pub(crate) type SharedPolicy = Rc<RefCell<Option<Arc<dyn FetchPolicy>>>>;

/// The plugin whose script `lua` runs; `None` for the app's own.
pub(crate) fn caller_of(lua: &mlua::Lua) -> Option<Rc<str>> {
    let env = lua.app_data_ref::<crate::node::Env>()?;
    match &*env.grants {
        crate::Grants::All => None,
        crate::Grants::Only(_) => Some(env.owner.clone()),
    }
}
