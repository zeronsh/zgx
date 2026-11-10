//! The test harness: zgx-script's public testing kit (`testing::TestApp`),
//! which app authors use too, so these tests keep it honest. A local type
//! around it, so a test file can add helpers of its own.
use zgx::{compose::View, reactive::Runtime};
use zgx_script::{ScriptHost, testing::TestApp};

#[allow(dead_code)]
pub struct Harness(pub TestApp);
impl std::ops::Deref for Harness {
    type Target = TestApp;
    fn deref(&self) -> &TestApp {
        &self.0
    }
}
impl std::ops::DerefMut for Harness {
    fn deref_mut(&mut self) -> &mut TestApp {
        &mut self.0
    }
}
#[allow(dead_code)]
impl Harness {
    pub fn new(host: impl FnOnce(&Runtime) -> ScriptHost) -> Self {
        Self(TestApp::new(host))
    }
    pub fn inline(code: &str) -> Self {
        Self(TestApp::inline(code))
    }
    pub fn with_desktop(host: impl FnOnce(&Runtime) -> ScriptHost) -> Self {
        Self(TestApp::with_desktop(host))
    }
    pub fn inspected(
        host: impl FnOnce(&Runtime) -> ScriptHost,
        inspector: &zgx::inspect::Inspector,
    ) -> Self {
        Self(TestApp::inspected(host, inspector))
    }
    pub fn wrapped(
        host: impl FnOnce(&Runtime) -> ScriptHost,
        wrap: impl FnOnce(View) -> View,
    ) -> Self {
        Self(TestApp::wrapped(host, wrap))
    }
}

/// A directory of the test's own in the temp directory,
/// `zgx-<name>-<pid>-<n>`, made now and removed as the test binary ends,
/// a failing test's too (libtest catches its panic; the process exits as
/// usual). Only what the tests made goes: never a sweep of the temp
/// directory by name.
#[allow(dead_code)]
pub fn temp_dir(name: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "zgx-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    made(&dir);
    dir
}
/// As `temp_dir`, but named `name` exactly in the temp directory: for
/// tests whose files reach one another's by name (a `.luaurc`'s
/// `"../zgx-packages-…"`). Made fresh, and removed as the binary ends.
#[allow(dead_code)]
pub fn temp_dir_named(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    made(&dir);
    dir
}
/// `dir` made, and listed to remove as the test binary ends.
fn made(dir: &std::path::Path) {
    use std::sync::{Mutex, Once, PoisonError};
    static MADE: Mutex<Vec<std::path::PathBuf>> = Mutex::new(Vec::new());
    static HOOK: Once = Once::new();
    unsafe extern "C" {
        fn atexit(hook: extern "C" fn()) -> std::ffi::c_int;
    }
    extern "C" fn remove_made() {
        let made = std::mem::take(&mut *MADE.lock().unwrap_or_else(PoisonError::into_inner));
        for dir in made {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
    // SAFETY: the C runtime's `atexit`, given a function that takes no
    // arguments and returns, registered once.
    HOOK.call_once(|| unsafe {
        atexit(remove_made);
    });
    std::fs::create_dir_all(dir).unwrap();
    MADE.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(dir.to_path_buf());
}
