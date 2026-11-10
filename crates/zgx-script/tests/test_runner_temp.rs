//! `zgx test` leaves the temp directory as it found it: each test's storage
//! (`ui.persist`'s) and each mount's files go as the test ends, a failing
//! test's included, and so does what an app the file kept past its tests
//! writes as it goes (its last save). Its own process: it points `TMPDIR` at
//! a directory of its own to count what lies there.
use std::path::Path;

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
    let root = std::env::temp_dir().join(format!("zgx-runner-temp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (temp, app) = (root.join("temp"), root.join("app"));
    std::fs::create_dir_all(&temp).unwrap();
    std::fs::create_dir_all(app.join("tests")).unwrap();
    // SAFETY: the only test of its process, before anything reads it.
    unsafe { std::env::set_var("TMPDIR", &temp) };
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
