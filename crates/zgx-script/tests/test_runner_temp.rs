//! `zgx test` leaves the temp directory as it found it: each test's storage
//! (`ui.persist`'s) and each mount's files go as the test ends, a failing
//! test's included, and so does what an app the file kept past its tests
//! writes as it goes (its last save). Its own process: it points `TMPDIR` at
//! a directory of its own to count what lies there.
use std::path::Path;

/// One test at a time: the first points the process's temp directory at
/// its own, where the others would then make theirs.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_test_run_leaves_the_temp_directory_as_it_found_it() {
    let _one = one_at_a_time();
    let root = std::env::temp_dir().join(format!("zgx-runner-temp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (temp, app) = (root.join("temp"), root.join("app"));
    std::fs::create_dir_all(&temp).unwrap();
    std::fs::create_dir_all(app.join("tests")).unwrap();
    // SAFETY: before anything of this test reads them; the other tests of
    // this process name their own child's (`TMPDIR` on Unix, `TMP` and
    // `TEMP` on Windows, which `temp_dir` reads there).
    unsafe {
        std::env::set_var("TMPDIR", &temp);
        std::env::set_var("TMP", &temp);
        std::env::set_var("TEMP", &temp);
    }
    assert_eq!(
        std::env::temp_dir().canonicalize().unwrap(),
        temp.canonicalize().unwrap()
    );
    std::fs::write(
        app.join("app.luau"),
        r##"local count = ui.persist("count", 0)
return ui.button {
	id = "add",
	on_click = function()
		count:set(count:get() + 1)
	end,
	"Add",
}"##,
    )
    .unwrap();
    std::fs::write(
        app.join("tests/app.test.luau"),
        r##"-- Kept past its tests: it saves as the file's VM goes.
local kept = mount()
kept:click("#add")

test("saves, writes a file and passes", function()
	local app = mount()
	app:click("#add")
	app:write_file("notes.txt", "hello")
	expect(app:read_file("notes.txt")).to_be("hello")
end)

test("saves, writes a file and fails", function()
	local app = mount()
	app:click("#add")
	app:write_file("notes.txt", "hello")
	expect(app:state("count")).to_be(99)
end)
"##,
    )
    .unwrap();
    let before = entries(&temp);
    let report = zgx_script::test_runner::run(&app, None, |_| {}).unwrap();
    assert_eq!(report.outcomes.len(), 2, "{:?}", report.outcomes);
    assert_eq!(
        report.failed(),
        1,
        "the second fails: {:?}",
        report.outcomes
    );
    assert_eq!(
        entries(&temp),
        before,
        "what the run left in the temp directory"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ctrl-C mid-test: `zgx test` removes what its tests made before it ends.
#[cfg(unix)]
#[test]
fn an_interrupted_run_leaves_nothing_either() {
    let _one = one_at_a_time();
    let root = std::env::temp_dir().join(format!("zgx-runner-interrupt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (temp, app) = (root.join("temp"), root.join("app"));
    std::fs::create_dir_all(&temp).unwrap();
    std::fs::create_dir_all(app.join("tests")).unwrap();
    std::fs::write(
        app.join("app.luau"),
        r##"local count = ui.persist("count", 0)
return ui.button { id = "add", on_click = function() count:set(count:get() + 1) end, "Add" }"##,
    )
    .unwrap();
    std::fs::write(
        app.join("tests/app.test.luau"),
        r##"test("runs until interrupted", function()
	local app = mount()
	app:click("#add")
	app:write_file("notes.txt", "hello")
	local start = os.clock()
	while os.clock() - start < 60 do
	end
end)
"##,
    )
    .unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_zgx"))
        .arg("test")
        .arg(&app)
        .env("TMPDIR", &temp)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    // Interrupted once its test has made its files' directory (its storage
    // is written as the app next saves, which a test busy to the end never
    // lets it).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while entries(&temp).is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the test made {:?}",
            entries(&temp)
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // SAFETY: a signal to the child this test started.
    unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(130), "{status:?}");
    assert_eq!(
        entries(&temp),
        Vec::<String>::new(),
        "what the interrupted run left"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ctrl-C while a test keeps writing its files: what it writes after the
/// cleanup began is never left. The cleanup holds the run's list until the
/// process ends, and a test writes only under it, so a write waits rather
/// than making the test's directory again. (`test-hooks`: a window between
/// the cleanup and the end, which a test writing on would land in.)
#[cfg(all(unix, feature = "test-hooks"))]
#[test]
fn an_interrupt_while_a_test_writes_leaves_nothing() {
    let _one = one_at_a_time();
    let root = std::env::temp_dir().join(format!("zgx-runner-writing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (temp, app) = (root.join("temp"), root.join("app"));
    std::fs::create_dir_all(&temp).unwrap();
    std::fs::create_dir_all(app.join("tests")).unwrap();
    std::fs::write(app.join("app.luau"), "return ui.text { \"hello\" }").unwrap();
    std::fs::write(
        app.join("tests/app.test.luau"),
        r##"test("writes until interrupted", function()
	local app = mount()
	local start = os.clock()
	local n = 0
	while os.clock() - start < 60 do
		n += 1
		app:write_file(`notes{n % 4}.txt`, "hello")
	end
end)
"##,
    )
    .unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_zgx"))
        .arg("test")
        .arg(&app)
        .env("TMPDIR", &temp)
        .env("ZGX_TEST_EXIT_DELAY_MS", "400")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while entries(&temp).is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the test made {:?}",
            entries(&temp)
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // SAFETY: a signal to the child this test started.
    unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(130), "{status:?}");
    assert_eq!(
        entries(&temp),
        Vec::<String>::new(),
        "what the run left, writing as it was interrupted"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ctrl-Break mid-test (Windows): `zgx test` removes what its tests made
/// before it ends, with a console interrupt's code. The run starts in a
/// process group of its own, which a console event can name; Ctrl-C can't
/// be sent to one, Ctrl-Break can, and the handler takes both alike.
#[cfg(windows)]
#[test]
fn an_interrupted_run_leaves_nothing_on_windows() {
    let _one = one_at_a_time();
    use std::os::windows::process::CommandExt;
    use windows::Win32::System::Console::{CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent};
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x200;
    let root = std::env::temp_dir().join(format!("zgx-runner-break-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (temp, app) = (root.join("temp"), root.join("app"));
    std::fs::create_dir_all(&temp).unwrap();
    std::fs::create_dir_all(app.join("tests")).unwrap();
    std::fs::write(app.join("app.luau"), "return ui.text { \"x\" }").unwrap();
    std::fs::write(
        app.join("tests/app.test.luau"),
        r##"test("runs until interrupted", function()
	local app = mount()
	app:write_file("notes.txt", "hello")
	local start = os.clock()
	while os.clock() - start < 60 do
	end
end)
"##,
    )
    .unwrap();
    // A console event reaches only a group on the sender's own console: a
    // test run without one (a service, some CI runners) makes one first,
    // which the child, started after, shares.
    // SAFETY: console calls with no arguments to keep alive; a console made
    // here is this process's until it ends.
    unsafe {
        if windows::Win32::System::Console::GetConsoleWindow().is_invalid() {
            let _ = windows::Win32::System::Console::AllocConsole();
        }
    }
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_zgx"))
        .arg("test")
        .arg(&app)
        .env("TMP", &temp)
        .env("TEMP", &temp)
        .creation_flags(CREATE_NEW_PROCESS_GROUP)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while entries(&temp).is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the test made nothing"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // SAFETY: an event for the process group this test started.
    unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, child.id()) }.unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0xC000_013A_u32 as i32), "{status:?}");
    assert_eq!(
        entries(&temp),
        Vec::<String>::new(),
        "what the interrupted run left"
    );
    let _ = std::fs::remove_dir_all(&root);
}
