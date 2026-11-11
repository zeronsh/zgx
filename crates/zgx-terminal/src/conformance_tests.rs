//! What programs ask of a terminal and rely on, from their side: each sends
//! the sequences a TUI does (an agent's CLI, an editor, a pager, a prompt),
//! and what the terminal answers, shows and sends back is checked. No real
//! programs: the sequences are fed to a replay session, so this runs as fast
//! on every platform's CI as the rest of the crate's tests.
use super::{Msg, Session, SessionEvent, SessionOptions};
use crate::keys::{self, Mouse};
use alacritty_terminal::{
    grid::Dimensions,
    index::{Column, Line},
    term::TermMode,
    vte::ansi::CursorShape,
};
use zgx::input::{Modifiers, PointerButton};

/// A replay session 40 by 10, and what it writes back to its program.
fn session() -> (Session, std::sync::mpsc::Receiver<Msg>) {
    let size = crate::GridSize::new(40, 10);
    Session::replay_with(&SessionOptions::default(), size, || {}, true)
}

/// What the session wrote back since the last call, joined.
fn written(rx: &std::sync::mpsc::Receiver<Msg>) -> String {
    rx.try_iter()
        .filter_map(|msg| match msg {
            Msg::Input(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            _ => None,
        })
        .collect()
}

/// `output` fed to a new session, and what it answered.
fn answer(output: &[u8]) -> String {
    let (session, rx) = session();
    session.feed(output);
    written(&rx)
}

/// A screen row's text, trailing spaces left out.
fn row(session: &Session, line: i32) -> String {
    let term = session.term();
    let grid = term.grid();
    let text: String = (0..grid.columns())
        .map(|column| grid[Line(line)][Column(column)].c)
        .collect();
    text.trim_end().to_owned()
}

/// The private modes TUIs set and ask about (DECRQM): each reported as it
/// is, set (1) or reset (2), before and after the program sets and resets it.
/// The cursor's visibility (25) starts set; the rest start reset.
#[test]
fn private_modes_report_as_set_and_reset() {
    let modes = [1, 25, 1000, 1002, 1003, 1004, 1006, 1049, 2004];
    for mode in modes {
        let (session, rx) = session();
        let ask = format!("\x1b[?{mode}$p");
        let starts = if mode == 25 { 1 } else { 2 };
        session.feed(ask.as_bytes());
        assert_eq!(
            written(&rx),
            format!("\x1b[?{mode};{starts}$y"),
            "{mode} at first"
        );
        session.feed(format!("\x1b[?{mode}h{ask}").as_bytes());
        assert_eq!(written(&rx), format!("\x1b[?{mode};1$y"), "{mode} set");
        session.feed(format!("\x1b[?{mode}l{ask}").as_bytes());
        assert_eq!(written(&rx), format!("\x1b[?{mode};2$y"), "{mode} reset");
    }
}

/// Synchronized output (2026): reported reset, so a program knows it can
/// hold a frame; what it asks inside an update is answered as the update
/// ends (the bytes after `2026h` wait with the frame, and are read once it
/// is shown, the mode reset by then).
#[test]
fn synchronized_output_reports_and_answers_as_the_update_ends() {
    let (session, rx) = session();
    session.feed(b"\x1b[?2026$p");
    assert_eq!(written(&rx), "\x1b[?2026;2$y", "reset at first");
    session.feed(b"\x1b[?2026h\x1b[?2026$p");
    assert_eq!(written(&rx), "", "held with the frame");
    session.feed(b"\x1b[?2026l");
    assert_eq!(written(&rx), "\x1b[?2026;2$y", "answered as it ends");
    session.feed(b"\x1b[?2026$p");
    assert_eq!(written(&rx), "\x1b[?2026;2$y", "reset after");
}

/// The kitty keyboard protocol's flags (agent CLIs and editors push them to
/// tell Escape, Tab and Enter apart from their control keys): reported,
/// pushed and popped.
#[test]
fn kitty_keyboard_flags_push_report_and_pop() {
    let (session, rx) = session();
    session.feed(b"\x1b[?u");
    assert_eq!(written(&rx), "\x1b[?0u", "none at first");
    session.feed(b"\x1b[>1u\x1b[?u");
    assert_eq!(written(&rx), "\x1b[?1u", "pushed");
    session.feed(b"\x1b[>11u\x1b[?u");
    assert_eq!(written(&rx), "\x1b[?11u", "pushed again");
    session.feed(b"\x1b[<u\x1b[?u");
    assert_eq!(written(&rx), "\x1b[?1u", "popped to the one before");
    session.feed(b"\x1b[<u\x1b[?u");
    assert_eq!(written(&rx), "\x1b[?0u", "popped to none");
}

/// The reports programs probe the terminal with before they draw: device
/// attributes (sixel announced while pictures can arrive), status, the
/// cursor where it is, and the text area in characters.
#[test]
fn device_and_status_reports() {
    assert_eq!(answer(b"\x1b[c"), "\x1b[?62;4;22c");
    let secondary = answer(b"\x1b[>c");
    assert!(
        secondary.starts_with("\x1b[>0;") && secondary.ends_with(";1c"),
        "{secondary:?}"
    );
    assert_eq!(answer(b"\x1b[5n"), "\x1b[0n");
    assert_eq!(answer(b"\x1b[3;5H\x1b[6n"), "\x1b[3;5R");
    assert_eq!(answer(b"\x1b[18t"), "\x1b[8;10;40t");
}

/// The terminfo capabilities a program asks the terminal itself for
/// (XTGETTCAP), as nvim does for direct colour without `COLORTERM`: each found
/// one answered with its value hex encoded, a flag with none, one not known
/// as not found, and every answer in order and its own.
#[test]
fn terminfo_capabilities_asked_of_the_terminal() {
    let hex = |text: &str| -> String { text.bytes().map(|b| format!("{b:02X}")).collect() };
    let found = |name: &str, value: Option<&str>| match value {
        Some(value) => format!("\x1bP1+r{}={}\x1b\\", hex(name), hex(value)),
        None => format!("\x1bP1+r{}\x1b\\", hex(name)),
    };
    let ask = format!(
        "\x1bP+q{};{};{};{}\x1b\\\x1bP+q{}\x1b\\",
        hex("Tc"),
        hex("RGB"),
        hex("setrgbf"),
        hex("nope"),
        hex("Ms"),
    );
    assert_eq!(
        answer(ask.as_bytes()),
        [
            found("Tc", None),
            found("RGB", None),
            found("setrgbf", Some("\\E[38;2;%p1%d;%p2%d;%p3%dm")),
            format!("\x1bP0+r{}\x1b\\", hex("nope")),
            found("Ms", Some("\\E]52;%p1%s;%p2%s\\007")),
        ]
        .concat()
    );
    assert_eq!(
        answer(format!("\x1bP+q{}\x1b\\", hex("TN")).as_bytes()),
        found("TN", Some("xterm-256color")),
        "the name TERM gives"
    );
}

/// The terminal's name and version (XTVERSION), as programs ask before
/// trusting what `TERM` says (over ssh, in tmux): the program name and
/// version its command is given, the app's own when it sets them.
#[test]
fn the_terminal_names_itself() {
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(answer(b"\x1b[>q"), format!("\x1bP>|zgx({version})\x1b\\"));
    assert_eq!(answer(b"\x1b[>0q"), format!("\x1bP>|zgx({version})\x1b\\"));
    assert_eq!(answer(b"\x1b[>1q"), "", "only 0 asks");
    let options = SessionOptions {
        env: vec![
            ("TERM_PROGRAM".into(), "app".into()),
            ("TERM_PROGRAM_VERSION".into(), "2.1".into()),
        ],
        ..SessionOptions::default()
    };
    let (session, rx) = Session::replay_with(&options, crate::GridSize::new(40, 10), || {}, true);
    session.feed(b"\x1b[>q");
    assert_eq!(written(&rx), "\x1bP>|app(2.1)\x1b\\", "the app's own");
}

/// The mouse modes a program sets decide what a click sends it: nothing
/// until it asks, X10 bytes or SGR as it asks, motion only with a button
/// held (1002) or always (1003).
#[test]
fn mouse_modes_decide_what_a_click_sends() {
    let none = Modifiers::default();
    let (session, _rx) = session();
    let send = |event| keys::mouse(event, 3, 4, none, *session.term().mode());
    let press = Mouse::Press(PointerButton::Primary);
    assert_eq!(send(press), None, "not asked");
    session.feed(b"\x1b[?1000h");
    assert_eq!(send(press), Some(b"\x1b[M \x24\x25".to_vec()), "X10");
    session.feed(b"\x1b[?1006h");
    assert_eq!(send(press), Some(b"\x1b[<0;4;5M".to_vec()), "SGR press");
    let release = Mouse::Release(PointerButton::Primary);
    assert_eq!(send(release), Some(b"\x1b[<0;4;5m".to_vec()), "SGR release");
    let drag = Mouse::Move(Some(PointerButton::Primary));
    assert_eq!(send(drag), None, "no motion without 1002");
    session.feed(b"\x1b[?1002h");
    assert_eq!(send(drag), Some(b"\x1b[<32;4;5M".to_vec()), "a drag");
    assert_eq!(send(Mouse::Move(None)), None, "no bare motion without 1003");
    session.feed(b"\x1b[?1003h");
    assert_eq!(
        send(Mouse::Move(None)),
        Some(b"\x1b[<35;4;5M".to_vec()),
        "bare motion"
    );
    session.feed(b"\x1b[?1000l\x1b[?1002l\x1b[?1003l");
    assert_eq!(send(press), None, "all off again");
}

/// A paste is bracketed once the program asks (2004), so an agent's prompt
/// takes pasted lines as text, not as Enter after each.
#[test]
fn a_paste_is_bracketed_once_asked() {
    let (session, _rx) = session();
    let paste = |text| keys::paste(text, *session.term().mode());
    assert_eq!(paste("one\ntwo"), b"one\rtwo".to_vec());
    session.feed(b"\x1b[?2004h");
    assert_eq!(paste("one\ntwo"), b"\x1b[200~one\ntwo\x1b[201~".to_vec());
}

/// The alternate screen (1049): a full-screen program draws on a screen of
/// its own, and leaving it brings back what was there and the cursor.
#[test]
fn the_alternate_screen_gives_back_the_main_one() {
    let (session, _rx) = session();
    session.feed(b"$ ls\r\nmain");
    session.feed(b"\x1b[?1049h");
    assert_eq!(row(&session, 0), "", "a clear screen of its own");
    session.feed(b"\x1b[1;1Hfull screen");
    assert!(session.term().mode().contains(TermMode::ALT_SCREEN));
    session.feed(b"\x1b[?1049l");
    assert_eq!(row(&session, 0), "$ ls");
    assert_eq!(row(&session, 1), "main");
    let cursor = session.term().grid().cursor.point;
    assert_eq!(
        (cursor.line.0, cursor.column.0),
        (1, 4),
        "the cursor where it was"
    );
}

/// The cursor's shape a program sets (DECSCUSR): an editor's bar in insert
/// mode, a block in normal mode, an underline, and the default again.
#[test]
fn cursor_shapes_follow_what_the_program_sets() {
    let (session, _rx) = session();
    let shape = || session.term().cursor_style().shape;
    session.feed(b"\x1b[6 q");
    assert_eq!(shape(), CursorShape::Beam);
    session.feed(b"\x1b[2 q");
    assert_eq!(shape(), CursorShape::Block);
    session.feed(b"\x1b[4 q");
    assert_eq!(shape(), CursorShape::Underline);
    session.feed(b"\x1b[0 q");
    assert_eq!(shape(), CursorShape::Block, "the default");
}

/// The clipboard through OSC 52, as agent CLIs and editors copy over a
/// session: text a program sets reaches the app, and a query asks the app.
#[test]
fn the_clipboard_through_osc_52() {
    let (session, _rx) = session();
    session.feed(b"\x1b]52;c;aGVsbG8=\x07");
    let events = session.take_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, SessionEvent::ClipboardStore(text) if text == "hello")),
        "{events:?}"
    );
    session.feed(b"\x1b]52;c;?\x07");
    let events = session.take_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, SessionEvent::ClipboardLoad(_))),
        "{events:?}"
    );
}

/// A link a program prints (OSC 8) is on the cells it covers, and only
/// those.
#[test]
fn a_link_covers_its_cells() {
    let (session, _rx) = session();
    session.feed(b"\x1b]8;;https://zgx.dev\x07site\x1b]8;;\x07 after");
    let term = session.term();
    let link = |column| {
        term.grid()[Line(0)][Column(column)]
            .hyperlink()
            .map(|link| link.uri().to_owned())
    };
    assert_eq!(link(0).as_deref(), Some("https://zgx.dev"));
    assert_eq!(link(3).as_deref(), Some("https://zgx.dev"));
    assert_eq!(link(5), None, "not past it");
}
