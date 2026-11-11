//! Real programs on a real pseudoterminal: an editor, a pager, a process
//! viewer, a multiplexer, git's TUIs and prompts, a fuzzy finder, each
//! started by a session, driven by the keys the view would send and read
//! back from the screen. Ignored by default, as they need the programs
//! installed: `cargo test -p zgx-terminal --lib real_programs -- --ignored`
//! runs them. `ZGX_TUI_PATH` is put before `PATH`, for programs installed
//! beside the system's (a newer nvim, lazygit).
use super::{GridSize, Session, SessionOptions};
use crate::keys::{self, KeyEvent};
use alacritty_terminal::{
    grid::Dimensions,
    index::{Column, Line},
    term::TermMode,
};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use zgx::input::{Key, Modifiers};

/// A program running in a session 100 by 30, in a directory of its own,
/// with a `HOME` of its own beside it (no user's configuration, and none of
/// the program's written into the directory).
struct Run {
    session: Session,
    dir: PathBuf,
    home: PathBuf,
}

/// A directory of its own under the system's temporary one.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("zgx-tui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `PATH` with `ZGX_TUI_PATH` before it.
fn path() -> String {
    let system = std::env::var("PATH").unwrap_or_default();
    match std::env::var("ZGX_TUI_PATH") {
        Ok(extra) => format!("{extra}:{system}"),
        Err(_) => system,
    }
}

/// Whether `program` is on `path()`; a run of one that isn't fails, so a
/// probe can't pass by running nothing.
fn require(program: &str) {
    let found = path()
        .split(':')
        .any(|dir| Path::new(dir).join(program).is_file());
    assert!(found, "{program} isn't installed (ZGX_TUI_PATH, PATH)");
}

impl Run {
    /// `command` run by `/bin/sh -c` in `dir`.
    fn start(dir: PathBuf, command: &str) -> Self {
        Self::start_with(dir, command, |_| {})
    }

    /// `command` run by `/bin/sh -c` in `dir`, its home set up by `prepare`
    /// first.
    fn start_with(dir: PathBuf, command: &str, prepare: impl FnOnce(&Path)) -> Self {
        let home = dir.with_extension("home");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        prepare(&home);
        let options = SessionOptions {
            command: Some(command.into()),
            working_directory: Some(dir.clone()),
            env: vec![
                ("HOME".into(), home.display().to_string()),
                ("PATH".into(), path()),
                ("LANG".into(), "C.UTF-8".into()),
                ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ],
            ..SessionOptions::default()
        };
        let size = GridSize {
            cell_width: 16,
            cell_height: 32,
            ..GridSize::new(100, 30)
        };
        let session = Session::spawn(&options, size, || {}).expect("a session");
        Self { session, dir, home }
    }

    /// The screen's rows, trailing spaces left out.
    fn rows(&self) -> Vec<String> {
        let term = self.session.term();
        let grid = term.grid();
        (0..grid.screen_lines() as i32)
            .map(|line| {
                let row = &grid[Line(line)];
                let text: String = (0..grid.columns())
                    .map(|column| row[Column(column)].c)
                    .collect();
                text.trim_end().to_owned()
            })
            .collect()
    }

    fn screen(&self) -> String {
        self.rows().join("\n")
    }

    fn mode(&self) -> TermMode {
        *self.session.term().mode()
    }

    /// Wait (up to 20 seconds) for `ready` to hold of the screen, or fail
    /// showing it.
    fn until(&self, what: &str, ready: impl Fn(&Self) -> bool) {
        let start = Instant::now();
        while !ready(self) {
            if let Some(code) = self.session.exited() {
                if !ready(self) {
                    panic!("exited ({code}) before {what}:\n{}", self.screen());
                }
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "no {what} after 20 s:\n{}",
                self.screen()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Wait for `text` on the screen.
    fn shows(&self, text: &str) {
        self.until(&format!("{text:?}"), |run| run.screen().contains(text));
    }

    /// Text typed, as the view's text event sends it.
    fn type_text(&self, text: &str) {
        self.session.write(text.as_bytes().to_vec());
    }

    /// A key pressed with `modifiers`, as the view sends it: the kitty
    /// keyboard protocol's encoding once the program asks for it, xterm's
    /// otherwise.
    fn key_with(&self, key: Key, modifiers: Modifiers) {
        let mode = self.mode();
        let bytes = match keys::kitty(&key, modifiers, KeyEvent::Press, None, mode) {
            Some(kitty) => kitty,
            None => keys::encode(&key, modifiers, mode),
        };
        match bytes {
            Some(bytes) => self.session.write(bytes),
            // A key that types text: the text event's.
            None => match key {
                Key::Character(text) => self.type_text(&text),
                Key::Space => self.type_text(" "),
                key => panic!("{key:?} sends nothing"),
            },
        }
    }

    fn key(&self, key: Key) {
        self.key_with(key, Modifiers::default());
    }

    fn ctrl(&self, character: &str) {
        let control = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        self.key_with(Key::Character(character.into()), control);
    }

    /// Wait for the program to end, and its exit code.
    fn exits(&self) -> i32 {
        let start = Instant::now();
        loop {
            if let Some(code) = self.session.exited() {
                return code;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "still running after 20 s:\n{}",
                self.screen()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// A file of `lines` numbered lines.
fn numbered(dir: &Path, name: &str, lines: usize) {
    let text: String = (1..=lines).map(|n| format!("line {n}\n")).collect();
    std::fs::write(dir.join(name), text).unwrap();
}

/// git, quietly, in `dir`.
fn git(dir: &Path, arguments: &[&str]) {
    let status = std::process::Command::new("git")
        .args(["-c", "user.name=zgx", "-c", "user.email=zgx@example.com"])
        .args(arguments)
        .current_dir(dir)
        .env("HOME", dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git");
    assert!(status.status.success(), "git {arguments:?}: {status:?}");
}

/// A repository with one commit, and a change to its file.
fn repository(name: &str) -> PathBuf {
    let dir = scratch(name);
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.name", "zgx"]);
    git(&dir, &["config", "user.email", "zgx@example.com"]);
    numbered(&dir, "notes.txt", 20);
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "first"]);
    let changed = std::fs::read_to_string(dir.join("notes.txt"))
        .unwrap()
        .replace("line 10\n", "line ten\n");
    std::fs::write(dir.join("notes.txt"), changed).unwrap();
    dir
}

/// vim: the alternate screen while it runs, text typed in insert mode,
/// `:wq` writing the file, and the shell's screen back once it ends.
fn edits_and_writes(editor: &str, arguments: &str) {
    require(editor);
    let dir = scratch(editor);
    let run = Run::start(
        dir.clone(),
        &format!("printf 'before\\n'; {editor} {arguments} notes.txt"),
    );
    run.until("the editor's screen", |run| {
        run.mode().contains(TermMode::ALT_SCREEN) && run.screen().contains('~')
    });
    run.type_text("i");
    run.type_text("hello from zgx");
    run.key(Key::Escape);
    run.type_text(":wq");
    run.key(Key::Enter);
    assert_eq!(run.exits(), 0, "{}", run.screen());
    assert_eq!(
        std::fs::read_to_string(dir.join("notes.txt")).unwrap(),
        "hello from zgx\n"
    );
    assert!(!run.mode().contains(TermMode::ALT_SCREEN));
    assert_eq!(run.rows()[0], "before", "the shell's screen back");
}

#[test]
#[ignore = "runs vim, which may not be installed"]
fn vim_edits_and_writes() {
    edits_and_writes("vim", "-u NONE -N -i NONE");
}

#[test]
#[ignore = "runs nvim, which may not be installed"]
fn nvim_edits_and_writes() {
    edits_and_writes("nvim", "--clean");
}

/// nvim, where `COLORTERM` isn't set (over ssh), asks the terminal for
/// direct colour (XTGETTCAP, then DECRQSS) and turns its own colours on
/// once it hears it can.
#[test]
#[ignore = "runs nvim, which may not be installed"]
fn nvim_hears_direct_colour() {
    require("nvim");
    let dir = scratch("nvim-caps");
    let run = Run::start(
        dir.clone(),
        "env -u COLORTERM nvim --clean +'lua vim.defer_fn(function() \
         vim.fn.writefile({tostring(vim.o.termguicolors)}, \"caps\") end, 1000)'",
    );
    run.until("nvim's screen", |run| run.screen().contains('~'));
    let start = Instant::now();
    while !dir.join("caps").exists() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "{}",
            run.screen()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    run.type_text(":qa!");
    run.key(Key::Enter);
    run.exits();
    assert_eq!(
        std::fs::read_to_string(dir.join("caps")).unwrap().trim(),
        "true",
        "termguicolors"
    );
}

/// less: a page at a time, a search, and the screen given back.
#[test]
#[ignore = "runs less, which may not be installed"]
fn less_pages_and_searches() {
    require("less");
    let dir = scratch("less");
    numbered(&dir, "long.txt", 200);
    let run = Run::start(dir, "printf 'before\\n'; less long.txt");
    run.shows("line 29");
    assert_eq!(run.rows()[0], "line 1");
    run.key(Key::Space);
    run.until("the next page", |run| run.rows()[0] == "line 30");
    run.type_text("/line 150");
    run.key(Key::Enter);
    run.until("the match at the top", |run| run.rows()[0] == "line 150");
    run.key(Key::End);
    run.shows("(END)");
    run.type_text("q");
    assert_eq!(run.exits(), 0);
    assert_eq!(run.rows()[0], "before", "the shell's screen back");
}

/// htop: its meters and process table drawn, sorted by a key, and quit.
#[test]
#[ignore = "runs htop, which may not be installed"]
fn htop_draws_and_quits() {
    require("htop");
    let run = Run::start(scratch("htop"), "htop");
    run.shows("PID");
    run.shows("Load average");
    run.key(Key::Function(10));
    assert_eq!(run.exits(), 0, "{}", run.screen());
}

/// tmux: a session inside the terminal, its status line on the last row,
/// a window split, and a command's output in each pane.
#[test]
#[ignore = "runs tmux, which may not be installed"]
fn tmux_splits_and_shows_both_panes() {
    require("tmux");
    let dir = scratch("tmux");
    let socket = dir.join("socket");
    let run = Run::start(
        dir,
        &format!(
            "tmux -S {} -f /dev/null new-session 'echo left side; exec sh'",
            socket.display()
        ),
    );
    run.shows("left side");
    run.until("the status line", |run| run.rows()[29].contains("[0]"));
    run.ctrl("b");
    run.type_text("%");
    run.until("a second pane", |run| run.screen().contains('│'));
    run.type_text("echo right side\r");
    run.shows("right side");
    let rows = run.rows();
    let left = rows.iter().any(|row| row.starts_with("left side"));
    let right = rows.iter().any(|row| {
        row.find("right side")
            .is_some_and(|at| row[..at].contains('│'))
    });
    assert!(left && right, "{}", run.screen());
    run.type_text("tmux -S ");
    run.type_text(&format!("{} kill-server\r", socket.display()));
    run.exits();
}

/// lazygit: its panels over a repository, a file staged with Space, a
/// commit typed into its prompt.
#[test]
#[ignore = "runs lazygit, which may not be installed"]
fn lazygit_stages_and_commits() {
    require("lazygit");
    let dir = repository("lazygit");
    // No first run's welcome or release notes over the panels: they show
    // while the state's versions are below lazygit's.
    let run = Run::start_with(dir.clone(), "lazygit", |home| {
        let state = home.join(".local/state/lazygit");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            state.join("state.yml"),
            "startuppopupversion: 999\nlastversion: 99.0.0\n",
        )
        .unwrap();
    });
    run.shows("Status");
    run.shows("M notes.txt");
    // The files panel has the focus as lazygit starts.
    run.key(Key::Space);
    run.type_text("c");
    run.shows("Commit summary");
    run.type_text("from lazygit");
    run.key(Key::Enter);
    run.until("the commit", |run| run.screen().contains("from lazygit"));
    run.type_text("q");
    assert_eq!(run.exits(), 0, "{}", run.screen());
    let log = std::process::Command::new("git")
        .args(["log", "--format=%s"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&log.stdout),
        "from lazygit\nfirst\n"
    );
}

/// `git add -p`: a hunk shown in colour and staged with `y`.
#[test]
#[ignore = "runs git, which may not be installed"]
fn git_add_patch_stages_a_hunk() {
    require("git");
    let dir = repository("add-p");
    let run = Run::start(dir.clone(), "git -c color.ui=always add -p");
    run.shows("+line ten");
    run.shows("Stage this hunk");
    run.type_text("y\r");
    assert_eq!(run.exits(), 0, "{}", run.screen());
    let staged = std::process::Command::new("git")
        .args(["diff", "--cached", "--stat"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&staged.stdout).contains("notes.txt"));
}

/// fzf: a list filtered as a query is typed, and the choice printed.
#[test]
#[ignore = "runs fzf, which may not be installed"]
fn fzf_filters_and_chooses() {
    require("fzf");
    let dir = scratch("fzf");
    let run = Run::start(
        dir.clone(),
        "printf 'apple\\nbanana\\ncherry\\n' | fzf --height 40% > chosen; echo done",
    );
    run.shows("3/3");
    run.type_text("chy");
    run.shows("1/3");
    run.key(Key::Enter);
    run.exits();
    assert_eq!(
        std::fs::read_to_string(dir.join("chosen")).unwrap(),
        "cherry\n"
    );
    assert!(run.screen().contains("done"));
}
