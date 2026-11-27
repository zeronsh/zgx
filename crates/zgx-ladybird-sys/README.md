# zgx-ladybird-sys

zgx's Ladybird host, `zgx-ladybird-host`: the process that runs a zgx app's Ladybird pages (web views whose options say `engine: WebEngine::Ladybird`, Luau `engine = "ladybird"`). It links Ladybird's LibWebView, which starts Ladybird's own helpers (WebContent, the Compositor, RequestServer, ImageDecoder, ...), and talks to the app over one socket (`host/Protocol.h`; zgx-desktop's `native_host/ladybird/protocol.rs` is its twin). A crash in Ladybird ends the app's Ladybird pages (`WebViewEvent::ProcessTerminated`), not the app.

Nothing of Ladybird's source is in zgx. `ladybird/build.py` (run it directly, or build this crate with its `build` feature, which names the install in `install_dir()`) fetches Ladybird at its pinned release, **2026.47.8** (`0be55a57cc07ed258362c631a13a10bb4f6d707a`), and vcpkg at the baseline Ladybird's manifest names, and builds them unmodified with the host in place of Ladybird's Qt browser:

- `ladybird/prepare.py` writes the vcpkg manifest the build uses: Ladybird's, without what would bring a copyleft licence into an app (cpptrace and libdwarf, Qt, Sparkle; libpsl uses ICU, not libidn2 and libunistring). It refuses a build whose plan or install has one of those, or whose FFmpeg isn't LGPL-2.1 (FFmpeg's configure line and its installed copyright file). It also places Chromium's HSTS preload list, pinned to a commit and checked against its hash, where Ladybird's build reads it, so two builds of one release make the same engine.
- `ladybird/zgx-project-include.cmake` (Ladybird's `CMAKE_PROJECT_ladybird_INCLUDE`) builds `host/` where Ladybird adds its `UI/`, as the target `ladybird`; `ladybird/qt-stub` answers Ladybird's configuration's search for Qt, which nothing then uses.
- The install (`cmake --install --component zgx_ladybird`) has `bin/zgx-ladybird-host`, the helpers in `libexec/`, libraries in `lib/`, resources in `share/Lagom/` (`MacOS/`, `Resources/` and `lib/` on macOS), and `LICENSES.txt`. The build fails if `LICENSES.txt` wasn't made.

The first build took about 50 minutes on 16 cores (vcpkg builds Skia, ICU, FFmpeg, curl and the rest, then Ladybird) and leaves about 8 GB (vcpkg 4.1 GB, the build 3.2 GB) and the 321 MB install under the cache: `ZGX_LADYBIRD_CACHE`, else `~/.cache/zgx-ladybird` (`~/Library/Caches/zgx-ladybird` on macOS). Later builds of the same release and host reuse its install. It needs git, CMake 3.30+, Ninja, a C++23 compiler (clang 20+ or GCC 16; Xcode 26 on macOS), Rust, Python 3, and what vcpkg's ports build with (nasm, autoconf, automake, libtool, pkg-config). Ladybird's CI image (`ghcr.io/ladybirdbrowser/ladybird-ci`) has them all.

Without the feature this crate builds nothing. It isn't published (`publish = false`) and zgx-desktop doesn't depend on it: zgx-desktop's `ladybird` feature finds a host at run time, in the install `ZGX_LADYBIRD_DIR` names, else in `zgx-ladybird` beside the app's executable, where an app ships it. Where there is none, Ladybird pages show their placeholders.

## Platforms

- **Linux**: built and tested (x86_64, Ladybird's CI image). Frames are the Compositor's own backing stores in shared memory, passed to the app once each (their descriptors) and then named per frame with what changed; the app uploads only that.
- **macOS**: the host builds its frames into shared memory of its own, copying what changed of each frame from the Compositor's IOSurfaces; importing those IOSurfaces directly is to come. Not yet built here.
- **Windows**: not yet. Ladybird's native Windows build is experimental; the host would need the socket as a named pipe, and the Compositor's Direct3D shared textures opened in the app.

## What an app with Ladybird pages ships

See [docs/ladybird.md](../../docs/ladybird.md#notices): `LICENSES.txt` from the install, and for the LGPL and MPL parts, what that page lists.
