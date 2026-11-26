//! zgx's Ladybird host: `zgx-ladybird-host`, a process that links Ladybird's
//! LibWebView and runs a zgx app's Ladybird pages (see README.md). With the
//! `build` feature, the build script fetches Ladybird at [`LADYBIRD_RELEASE`]
//! and builds it with the host; [`install_dir`] then names where it went.

/// The Ladybird release the host is built against, and its commit.
pub const LADYBIRD_RELEASE: (&str, &str) =
    ("2026.47.8", "0be55a57cc07ed258362c631a13a10bb4f6d707a");

/// Where the build script installed the host (`bin/zgx-ladybird-host`, its
/// helpers in `libexec`, libraries in `lib`, resources in `share/Lagom`, and
/// `LICENSES.txt`, the notices an app with Ladybird pages ships); `None`
/// without the `build` feature.
pub fn install_dir() -> Option<&'static str> {
    option_env!("ZGX_LADYBIRD_INSTALL_DIR")
}
