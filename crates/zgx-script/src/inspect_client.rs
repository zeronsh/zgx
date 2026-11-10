//! The other end of devtools ([`crate::inspect`, with the `inspect`
//! feature]): finding this user's running apps and asking one, for `zgx
//! inspect` and its MCP server.
use crate::Value;
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
};

/// The longest path a Unix socket binds, in bytes: `sockaddr_un`'s
/// `sun_path` less its closing NUL (104 bytes on Apple's systems, 108 on
/// Linux's).
#[cfg(unix)]
pub const SOCKET_PATH_MAX: usize = if cfg!(any(target_os = "linux", target_os = "android")) {
    107
} else {
    103
};

/// Where this user's apps put their sockets: a directory only they may
/// enter (`$TMPDIR/zgx-inspect-<uid>`, mode 0700; `%LOCALAPPDATA%` on
/// Windows). `ZGX_INSPECT_DIR` names another. Where a socket's path in the
/// temporary directory would be too long to bind (an iOS simulator app's
/// container is), `/tmp/zgx-inspect-<uid>` instead.
pub fn socket_dir() -> PathBuf {
    first_fitting(socket_dirs())
}

/// The first of `dirs` a socket binds in, else the first: its bind then
/// says why ([`socket_path`]).
fn first_fitting(dirs: Vec<PathBuf>) -> PathBuf {
    #[cfg(unix)]
    if let Some(dir) = dirs.iter().find(|dir| fits(dir)) {
        return dir.clone();
    }
    dirs.into_iter().next().expect("a socket directory")
}

/// Every directory an app may serve from, the one it tries first first:
/// `ZGX_INSPECT_DIR` alone when set, else the temporary directory's and,
/// on Unix, the short one beside it.
pub fn socket_dirs() -> Vec<PathBuf> {
    if let Some(dir) = zgx::env::var_os("ZGX_INSPECT_DIR").filter(|d| !d.is_empty()) {
        return vec![PathBuf::from(dir)];
    }
    #[cfg(unix)]
    {
        let uid = own_uid();
        let usual = std::env::temp_dir().join(format!("zgx-inspect-{uid}"));
        let short = PathBuf::from(format!("/tmp/zgx-inspect-{uid}"));
        if usual == short {
            vec![usual]
        } else {
            vec![usual, short]
        }
    }
    #[cfg(not(unix))]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        vec![base.join("zgx-inspect")]
    }
}

#[cfg(unix)]
fn own_uid() -> u32 {
    unsafe extern "C" {
        fn getuid() -> u32;
    }
    // SAFETY: getuid has no preconditions.
    unsafe { getuid() }
}

/// Whether any process's socket in `dir` binds: the longest process id's.
#[cfg(unix)]
fn fits(dir: &std::path::Path) -> bool {
    socket_path(dir, u32::MAX).is_ok()
}

/// Process `pid`'s socket in `dir`, or why it can't bind there: a path past
/// [`SOCKET_PATH_MAX`] fails to bind with no word of where.
#[cfg(unix)]
pub fn socket_path(dir: &std::path::Path, pid: u32) -> std::io::Result<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let path = dir.join(format!("{pid}.sock"));
    let length = path.as_os_str().as_bytes().len();
    if length > SOCKET_PATH_MAX {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "devtools' socket {} is {length} bytes, past the {SOCKET_PATH_MAX} a socket's path may be here: set ZGX_INSPECT_DIR to a shorter directory",
                path.display()
            ),
        ));
    }
    Ok(path)
}

/// A running app serving devtools: its process and what it said it is.
#[derive(Clone, Debug, PartialEq)]
pub struct App {
    /// Its process id.
    pub pid: u32,
    /// What it runs (its script).
    pub app: String,
}

/// The apps serving devtools now, by process id; files of apps gone are
/// left out (and removed).
pub fn apps() -> Vec<App> {
    let mut out: Vec<App> = Vec::new();
    for entry in socket_dirs()
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Some(about) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| Value::from_json(&t))
        else {
            continue;
        };
        let pid = about.field("pid").and_then(Value::as_f64).unwrap_or(0.) as u32;
        let app = about.field("app").map(Value::to_string).unwrap_or_default();
        if out.iter().any(|known| known.pid == pid) {
            continue;
        }
        if connect(pid).is_ok() {
            out.push(App { pid, app });
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
    out.sort_by_key(|a| a.pid);
    out
}

/// The app to ask: `pid`, or the only one running.
pub fn pick(pid: Option<u32>) -> Result<u32, String> {
    if let Some(pid) = pid {
        return Ok(pid);
    }
    let apps = apps();
    match apps.as_slice() {
        [one] => Ok(one.pid),
        [] => Err(
            "no app is serving devtools: run it with `zgx-run --inspect` (or ZGX_INSPECT_SOCKET=1)"
                .into(),
        ),
        many => Err(format!(
            "{} apps serve devtools; say which with --app:\n{}",
            many.len(),
            many.iter()
                .map(|a| format!("  {} {}", a.pid, a.app))
                .collect::<Vec<_>>()
                .join("\n")
        )),
    }
}

#[cfg(unix)]
type Stream = std::os::unix::net::UnixStream;
#[cfg(not(unix))]
type Stream = std::net::TcpStream;

#[cfg(unix)]
fn connect(pid: u32) -> std::io::Result<(Stream, Option<String>)> {
    // The directory it serves from: any of them holds its socket.
    let socket = socket_dirs()
        .into_iter()
        .map(|dir| dir.join(format!("{pid}.sock")))
        .find(|socket| socket.exists())
        .unwrap_or_else(|| socket_dir().join(format!("{pid}.sock")));
    Ok((Stream::connect(socket)?, None))
}
#[cfg(not(unix))]
fn connect(pid: u32) -> std::io::Result<(Stream, Option<String>)> {
    let about = std::fs::read_to_string(socket_dir().join(format!("{pid}.json")))?;
    let about = Value::from_json(&about).ok_or_else(|| std::io::Error::other("unreadable"))?;
    let port = about.field("port").and_then(Value::as_f64).unwrap_or(0.) as u16;
    let token = about.field("token").map(Value::to_string);
    Ok((Stream::connect(("127.0.0.1", port))?, token))
}

/// Ask app `pid` one request (a JSON object): its answer, a JSON line.
pub fn request(pid: u32, request: &Value) -> Result<String, String> {
    let (stream, token) = connect(pid).map_err(|e| format!("app {pid}: {e}"))?;
    let request = match (token, request) {
        (Some(token), Value::Map(fields)) => {
            let mut fields: Vec<_> = fields.iter().cloned().collect();
            fields.push(("token".into(), Value::from(token.as_str())));
            Value::record(fields)
        }
        (_, request) => request.clone(),
    };
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    writeln!(writer, "{}", request.to_json()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if line.is_empty() {
        return Err(format!("app {pid} closed the connection"));
    }
    Ok(line.trim_end().to_owned())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A directory whose sockets' paths come to `length` bytes for the
    /// longest process id.
    fn dir_for(length: usize) -> PathBuf {
        let name = format!("{}.sock", u32::MAX);
        PathBuf::from(format!("/{}", "d".repeat(length - name.len() - 2)))
    }

    #[test]
    fn a_socket_path_binds_up_to_the_platforms_limit_and_past_it_says_why() {
        assert!(socket_path(&dir_for(SOCKET_PATH_MAX), u32::MAX).is_ok());
        let error = socket_path(&dir_for(SOCKET_PATH_MAX + 1), u32::MAX).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        let message = error.to_string();
        assert!(message.contains("ZGX_INSPECT_DIR"), "{message}");
        assert!(message.contains(&SOCKET_PATH_MAX.to_string()), "{message}");
    }

    /// An iOS simulator app's temporary directory is about 150 bytes:
    /// its sockets go to the short directory, which the client reads too.
    #[test]
    fn a_temporary_directory_too_long_for_a_socket_falls_back_to_the_short_one() {
        let long = dir_for(SOCKET_PATH_MAX + 40);
        let short = PathBuf::from("/tmp/zgx-inspect-501");
        assert_eq!(first_fitting(vec![long.clone(), short.clone()]), short);
        let usual = PathBuf::from("/var/folders/xy/T/zgx-inspect-501");
        assert_eq!(first_fitting(vec![usual.clone(), short]), usual);
        // Nothing fits (an override too long): the first, whose bind says why.
        assert_eq!(first_fitting(vec![long.clone()]), long);
    }

    #[test]
    fn a_socket_in_the_short_directory_binds_and_answers() {
        use std::os::unix::net::{UnixListener, UnixStream};
        let dir = PathBuf::from(format!("/tmp/zgx-inspect-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = socket_path(&dir, u32::MAX).unwrap();
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let client = UnixStream::connect(&path);
        let accepted = listener.accept();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(client.is_ok() && accepted.is_ok());
    }
}
