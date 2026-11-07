//! `ui.ios`'s pieces as assistive technology meets them, against UIKit's.
mod common;

use common::Harness;
use zgx::semantics::{Role, SemanticNode};

/// Every semantic node with `role` assistive technology meets, in tree
/// order: none inside a hidden one (`aria_hidden`), as the tree leaves out.
fn with_role(h: &mut Harness, role: Role) -> Vec<SemanticNode> {
    h.ui.prepare_frame();
    let scene = h.ui.scene.borrow();
    let semantics = h.ui.semantics.borrow();
    let mut out = Vec::new();
    let mut pending = vec![scene.root()];
    while let Some(id) = pending.pop() {
        let node = semantics.get(id);
        if node.is_some_and(|n| n.hidden) {
            continue;
        }
        if let Some(node) = node.filter(|n| n.role == role) {
            out.push(node.clone());
        }
        pending.extend(scene.children(id).iter().rev().copied());
    }
    out
}

#[test]
fn a_row_reads_as_a_settings_cell_and_a_toggle_row_is_one_switch() {
    let mut h = Harness::inline(
        r#"local ios = ui.ios
        local on = ui.state("on", false)
        return ios.section {
            header = "Network",
            ios.row { title = "Wi-Fi", detail = "Home", icon = "wifi" },
            ios.row { title = "Airplane Mode", toggle = on, icon = "airplane" },
        }"#,
    );
    h.ui.set_assistive_technology(true);
    h.frames(2);
    assert_eq!(h.error(), None);
    // UIKit's cell: the title its label, the detail its value.
    let buttons = with_role(&mut h, Role::Button);
    let wifi = buttons
        .iter()
        .find(|n| n.label == "Wi-Fi")
        .expect("the Wi-Fi row");
    assert_eq!(wifi.value.as_deref(), Some("Home"));
    // With a switch, one element: the row, its state the switch's.
    let switches = with_role(&mut h, Role::Switch);
    assert_eq!(
        switches.len(),
        1,
        "one switch, not the row and its switch: {switches:?}"
    );
    assert_eq!(switches[0].label, "Airplane Mode");
    assert_eq!(switches[0].checked, Some(false));
    // Its header is a heading, as a table section's is (the headings rotor).
    let headings = with_role(&mut h, Role::Heading);
    assert!(
        headings.iter().any(|n| n.label == "Network"),
        "{headings:?}"
    );
}

impl Harness {
    /// Runs frames at 60 Hz until `ms` have passed.
    fn advance_ms(&mut self, ms: f64) {
        for _ in 0..(ms * 60. / 1000.).round() as usize {
            self.frame();
        }
    }
}

/// Whether node `id` is laid out: neither it nor what holds it is hidden.
fn displayed(h: &mut Harness, id: &str) -> bool {
    let node = h.find(id).unwrap_or_else(|| panic!("no node `{id}`"));
    let scene = h.ui.scene.borrow();
    std::iter::once(node).chain(scene.ancestors(node)).all(|n| {
        scene
            .style(n)
            .layout_options
            .as_deref()
            .and_then(|o| o.display)
            != Some(zgx::layout::Display::None)
    })
}

/// How much of node `id` shows: its opacity and every ancestor's.
fn seen(h: &mut Harness, id: &str) -> f32 {
    let node = h.find(id).unwrap_or_else(|| panic!("no node `{id}`"));
    let scene = h.ui.scene.borrow();
    std::iter::once(node)
        .chain(scene.ancestors(node))
        .map(|n| scene.effects(n).opacity)
        .product()
}

#[test]
fn with_reduce_motion_a_push_and_a_pop_cross_fade_as_uikits_do() {
    let mut h = Harness::inline(
        r#"local ios = ui.ios
        return ios.stack {
            root = function(nav)
                return ui.column {
                    ui.text { "Root", id = "root" },
                    ui.button { "Go", id = "go", on_click = function()
                        nav.push(function(inner)
                            return ui.button { "Back", id = "back", on_click = function() inner.pop() end }
                        end)
                    end },
                }
            end,
        }"#,
    );
    h.ui.theme()
        .set_system_preferences(zgx::theme::SystemPreferences {
            reduced_motion: true,
            ..Default::default()
        });
    h.frames(2);
    assert_eq!(h.error(), None);
    h.click("go");
    h.advance_ms(17.);
    // No slide: the new page is in place, fading in over the old.
    let early = seen(&mut h, "back");
    assert!(early < 0.3, "fading in from nothing: {early}");
    assert!(h.bounds("back").x.abs() < 0.5, "in place, not sliding in");
    h.advance_ms(250.);
    let (incoming, outgoing) = (seen(&mut h, "back"), seen(&mut h, "root"));
    assert!((0.25..0.85).contains(&incoming), "halfway in: {incoming}");
    assert!((0.15..0.75).contains(&outgoing), "halfway out: {outgoing}");
    h.advance_ms(400.);
    assert!((seen(&mut h, "back") - 1.).abs() < 0.01);
    assert!(
        !displayed(&mut h, "root"),
        "the page under goes once it has faded"
    );
    // A pop: the root fades back in, the page goes when it has faded.
    h.click("back");
    h.advance_ms(267.);
    let root = seen(&mut h, "root");
    assert!((0.25..0.85).contains(&root), "fading back in: {root}");
    h.advance_ms(400.);
    assert!(h.find("back").is_none(), "the popped page is gone");
    assert!((seen(&mut h, "root") - 1.).abs() < 0.01);
}

#[test]
fn a_sheets_grabber_toggles_its_detents_and_says_so_as_uikits() {
    let mut h = Harness::inline(
        r#"local open = ui.state("open", true)
        return ui.div { w = "100%", h = "100%",
            ui.ios.sheet { open = open, build = function() return ui.text { "Inside" } end },
        }"#,
    );
    h.ui.set_assistive_technology(true);
    h.frames(2);
    h.advance_ms(800.);
    assert_eq!(h.error(), None);
    let grabber = |h: &mut Harness| {
        with_role(h, Role::Button)
            .into_iter()
            .find(|n| n.label == "Sheet Grabber")
            .expect("the grabber")
    };
    let at_medium = grabber(&mut h);
    assert_eq!(at_medium.value.as_deref(), Some("Half screen"));
    assert_eq!(
        at_medium.description.as_deref(),
        Some("Double tap to expand the sheet.")
    );
    // A tap goes to the detent before, the largest after the smallest.
    let node = {
        let scene = h.ui.scene.borrow();
        let semantics = h.ui.semantics.borrow();
        let mut pending = vec![scene.root()];
        let mut found = None;
        while let Some(id) = pending.pop() {
            if semantics
                .get(id)
                .is_some_and(|n| n.label == "Sheet Grabber")
            {
                found = Some(id);
            }
            pending.extend(scene.children(id).iter().copied());
        }
        found.expect("the grabber's node")
    };
    h.ui.input
        .dispatch_to(&h.ui.scene, node, zgx::input::InputEvent::Activate);
    h.advance_ms(800.);
    let at_large = grabber(&mut h);
    assert_eq!(at_large.value.as_deref(), Some("Expanded"));
    assert_eq!(
        at_large.description.as_deref(),
        Some("Double tap to collapse the sheet.")
    );
}

/// UIKit's point sizes per text style at each content size category, extra
/// small to AX5, as Apple publishes them (Human Interface Guidelines,
/// Typography, "Dynamic Type sizes" for iOS). The runtime computes them in
/// CoreText's code: no table of them lies in its binaries to read, so these
/// are checked against `preferredFontForTextStyle:` on a device.
const CATEGORIES: [&str; 12] = [
    "xS", "S", "M", "L", "xL", "xxL", "xxxL", "AX1", "AX2", "AX3", "AX4", "AX5",
];
/// Footnote, subheadline, body, title 3, title 2 and large title: the
/// styles the iOS host gives `font.size.xs` to `font.size.xxl`, each with
/// the line height Apple gives it at the default size, in that ratio.
const STYLES: [(&str, &str, f32, [f32; 12]); 6] = [
    (
        "font.size.xs",
        "font.line.xs",
        18. / 13.,
        [12., 12., 12., 13., 15., 17., 19., 23., 27., 33., 38., 44.],
    ),
    (
        "font.size.sm",
        "font.line.sm",
        20. / 15.,
        [12., 13., 14., 15., 17., 19., 21., 25., 30., 36., 42., 49.],
    ),
    (
        "font.size.md",
        "font.line.md",
        22. / 17.,
        [14., 15., 16., 17., 19., 21., 23., 28., 33., 40., 47., 53.],
    ),
    (
        "font.size.lg",
        "font.line.lg",
        25. / 20.,
        [17., 18., 19., 20., 22., 24., 26., 31., 37., 43., 49., 55.],
    ),
    (
        "font.size.xl",
        "font.line.xl",
        28. / 22.,
        [19., 20., 21., 22., 24., 26., 28., 34., 39., 44., 50., 56.],
    ),
    (
        "font.size.xxl",
        "font.line.xxl",
        41. / 34.,
        [31., 32., 33., 34., 36., 38., 40., 44., 48., 52., 56., 60.],
    ),
];

/// The tokens the iOS host sets at content size category `index`
/// (`ios_system::tokens`): each style's size and line, and the hit target
/// growing with the body's line.
fn dynamic_type(index: usize) -> Vec<(zgx::theme::TokenId, zgx::theme::TokenInput)> {
    let mut tokens: Vec<(zgx::theme::TokenId, zgx::theme::TokenInput)> = STYLES
        .iter()
        .flat_map(|(size, line, ratio, sizes)| {
            let pt = sizes[index];
            [
                (zgx::theme::token(size), pt.into()),
                (zgx::theme::token(line), (pt * ratio).round().into()),
            ]
        })
        .collect();
    let (_, _, ratio, body) = STYLES[2];
    let body_line = (body[index] * ratio).round();
    tokens.push((
        zgx::theme::token("control.height"),
        (body_line + 12.).max(44.).into(),
    ));
    tokens
}

/// What shows of the app: every text's lines where it shows them, and the
/// boxes of its symbols and native controls, each with the ids of what holds
/// it. Unseen ones (transparent, hidden) are left out.
struct Painted {
    lines: Vec<(String, zgx::scene::Rect, zgx::scene::NodeId)>,
    marks: Vec<(zgx::scene::Rect, zgx::scene::NodeId)>,
}

fn painted(h: &mut Harness) -> Painted {
    use zgx::scene::NodeKind;
    h.frames(1);
    let scene = h.ui.scene.borrow();
    let semantics = h.ui.semantics.borrow();
    let holds_text = |id: zgx::scene::NodeId| {
        let mut pending = vec![id];
        while let Some(n) = pending.pop() {
            if matches!(scene.kind(n), NodeKind::Text { .. }) {
                return true;
            }
            pending.extend(scene.children(n).iter().copied());
        }
        false
    };
    let mut out = Painted {
        lines: Vec::new(),
        marks: Vec::new(),
    };
    let mut pending = vec![scene.root()];
    while let Some(id) = pending.pop() {
        let effects = scene.effects(id);
        let gone = effects.opacity <= 0.
            || scene
                .style(id)
                .layout_options
                .as_deref()
                .and_then(|o| o.display)
                == Some(zgx::layout::Display::None);
        if gone {
            continue;
        }
        // A switch's control (the row holding it is a switch too, with text).
        if semantics.get(id).is_some_and(|n| n.role == Role::Switch) && !holds_text(id) {
            out.marks
                .push((scene.window_rect(id, scene.bounds(id)), id));
            continue;
        }
        match scene.kind(id) {
            NodeKind::Text { text, .. } if !text.trim().is_empty() => {
                for line in scene.text_lines(id).unwrap_or_default() {
                    let words = text[line.range.clone()].to_string();
                    if line.rect.width > 0. && !words.trim().is_empty() {
                        out.lines.push((words, line.rect, id));
                    }
                }
            }
            NodeKind::Svg(_) | NodeKind::Image(_) | NodeKind::NativeSurface(_) => {
                out.marks
                    .push((scene.window_rect(id, scene.bounds(id)), id));
            }
            _ => {}
        }
        pending.extend(scene.children(id).iter().rev().copied());
    }
    out
}

fn overlaps(a: zgx::scene::Rect, b: zgx::scene::Rect) -> bool {
    // A hair's slack: lines that touch do not overlap.
    const SLACK: f32 = 0.5;
    a.x + a.width > b.x + SLACK
        && b.x + b.width > a.x + SLACK
        && a.y + a.height > b.y + SLACK
        && b.y + b.height > a.y + SLACK
}

/// Asserts what UIKit's cells keep at every text size: no text runs off the
/// screen's sides or out of a box that clips it (each wraps, or its box
/// grows), no line lies over another, and none over a symbol or control.
fn assert_fits(h: &mut Harness, width: f32, category: &str) {
    let shown = painted(h);
    let scene = h.ui.scene.borrow();
    for (words, rect, id) in &shown.lines {
        assert!(
            rect.x >= -0.5 && rect.x + rect.width <= width + 0.5,
            "{category}: `{words}` runs off the screen: {rect:?}"
        );
        // Inside every box above it that clips, as the box shows; down a
        // scroll view's content, the scroll view brings it into view.
        let mut scrolled = false;
        for holder in scene.ancestors(*id) {
            if scene.is_scroll_content(holder) {
                scrolled = true;
            }
            let (clip_x, clip_y) = scene.clips(holder);
            let clip_y = clip_y && !scrolled;
            if !(clip_x || clip_y) {
                continue;
            }
            let bounds = scene.window_rect(holder, scene.bounds(holder));
            let inside_x = !clip_x
                || (rect.x >= bounds.x - 0.5
                    && rect.x + rect.width <= bounds.x + bounds.width + 0.5);
            let inside_y = !clip_y
                || (rect.y >= bounds.y - 0.5
                    && rect.y + rect.height <= bounds.y + bounds.height + 0.5);
            assert!(
                inside_x && inside_y,
                "{category}: `{words}` is cut off by what holds it: {rect:?} in {bounds:?}"
            );
        }
    }
    for (i, (a_words, a, _)) in shown.lines.iter().enumerate() {
        for (b_words, b, _) in &shown.lines[i + 1..] {
            assert!(
                !overlaps(*a, *b),
                "{category}: `{a_words}` {a:?} lies over `{b_words}` {b:?}"
            );
        }
        for (mark, _) in &shown.marks {
            assert!(
                !overlaps(*a, *mark),
                "{category}: `{a_words}` {a:?} lies over a symbol or control at {mark:?}"
            );
        }
    }
}

#[test]
fn every_kit_screen_fits_its_text_at_each_dynamic_type_size_to_ax5() {
    // An iPhone 17 Pro's screen, in points.
    const W: f32 = 402.;
    const H: f32 = 874.;
    let mut h = Harness::inline(
        r#"local ios = ui.ios
        local Screen = ui.component(function()
        local on = ui.use_state(true)
        local query = ui.use_state("")
        local message = ui.use_state("")
        return ios.page {
            title = "Settings",
            ios.search { value = query },
            ios.section {
                header = "Connections and the devices this phone reaches",
                footer = "Your phone shares its connection with devices near it, as Personal Hotspot allows.",
                ios.row { title = "Wi-Fi", detail = "Home Network 5G", icon = "wifi" },
                ios.row { title = "Personal Hotspot", detail = "Off", icon = "personalhotspot" },
                ios.row { title = "Allow Notifications on the Lock Screen", toggle = on, icon = "bell" },
                ios.row { title = "Battery Health and Charging", detail = "Optimized", last = true },
            },
            ios.composer { value = message, bottom = 40 },
        }
        end)
        return Screen {}"#,
    );
    h.ui.scene.borrow_mut().resize(W, H);
    let viewport = h.ui.runtime.signal((W, H));
    h.host.window_size(viewport);
    h.frames(2);
    assert_eq!(h.error(), None);
    for (index, category) in CATEGORIES.iter().enumerate() {
        h.ui.theme().set_system_tokens(dynamic_type(index));
        // What eases to the new size (the composer's lines) comes to rest.
        h.advance_ms(1000.);
        assert_eq!(h.error(), None, "{category}");
        assert_fits(&mut h, W, category);
    }
}

#[test]
fn a_sheet_fits_its_rows_at_each_dynamic_type_size_to_ax5() {
    const W: f32 = 402.;
    const H: f32 = 874.;
    let mut h = Harness::inline(
        r#"local ios = ui.ios
        -- Opened once the window has its size, as an app's would be.
        local open = ui.state("open", false)
        return ui.div { w = "100%", h = "100%",
            ui.button { id = "open", w = 40, h = 40, on_click = function() open:set(true) end },
            ios.sheet { open = open, build = function()
                return ui.scroll { w = "100%", h = "100%",
                    ios.section {
                        header = "Shared with the people in this conversation",
                        ios.row { title = "Notifications", detail = "Mentions only" },
                        ios.row { title = "Hide Alerts for This Conversation", toggle = ui.use_state(false), last = true },
                    },
                }
            end },
        }"#,
    );
    h.ui.scene.borrow_mut().resize(W, H);
    let viewport = h.ui.runtime.signal((W, H));
    h.host.window_size(viewport);
    h.frames(2);
    h.click("open");
    h.advance_ms(800.);
    assert_eq!(h.error(), None);
    for (index, category) in CATEGORIES.iter().enumerate() {
        h.ui.theme().set_system_tokens(dynamic_type(index));
        // What eases to the new size (the composer's lines) comes to rest.
        h.advance_ms(1000.);
        assert_eq!(h.error(), None, "{category}");
        assert_fits(&mut h, W, category);
    }
}

/// A kit page of rows enough to scroll, its `keyboard_ride` as `ride` says.
fn page_riding(ride: &str) -> Harness {
    let h = Harness::inline(&format!(
        r#"local ios = ui.ios
        local Form = ui.component(function()
            local rows = {{}}
            for i = 1, 30 do
                rows[i] = ios.row {{ title = "Row " .. i }}
            end
            return ios.page {{ title = "Form", {ride} ios.section {{ table.unpack(rows) }} }}
        end)
        return Form {{}}"#
    ));
    h.ui.scene.borrow_mut().resize(402., 874.);
    h.frames(2);
    assert_eq!(h.error(), None);
    h
}

#[test]
fn a_page_rides_the_keyboard_as_uikits_by_default_or_attached_when_it_says() {
    use zgx::elastic::KeyboardRide;
    let ride = |h: &mut Harness| {
        h.ui.scroll_driver_at(200., 400.)
            .expect("the page's scroll view")
            .keyboard_ride()
    };
    // UIKit's own unless the page says otherwise.
    assert_eq!(ride(&mut page_riding("")), KeyboardRide::Native);
    assert_eq!(
        ride(&mut page_riding(r#"keyboard_ride = "native","#)),
        KeyboardRide::Native
    );
    assert_eq!(
        ride(&mut page_riding(r#"keyboard_ride = "attached","#)),
        KeyboardRide::Attached
    );
    let wrong = zgx_script::testing::TestApp::try_new(|runtime| {
        zgx_script::ScriptHost::inline(
            runtime,
            "app",
            r#"return ui.scroll { keyboard_ride = "glued" }"#,
        )
    });
    assert!(
        wrong
            .as_ref()
            .err()
            .is_some_and(|e| e.contains("keyboard_ride")),
        "{:?}",
        wrong.err()
    );
}

/// The keyboard as UIKit animates it (iOS 26.5, lab simulator): its own
/// `CASpringAnimation`, mass 1, from 336 points below where it rests.
const KEYBOARD_SPRING: (f64, f64, f64) = (1., 555.0265, 47.118);
const KEYBOARD_DURATION: f64 = 0.3833;
const KEYBOARD_TRAVEL: f64 = 336.;
/// Samples a second, as the iOS host's.
const RATE: f64 = 120.;
/// The room UIKit leaves under a field it scrolls into view.
const ROOM: f32 = 28.;

/// Where the keyboard's edge is at each sample, rising to `end`.
fn keyboard_edges(end: f64) -> Vec<f64> {
    let (mass, stiffness, damping) = KEYBOARD_SPRING;
    zgx::native_control::spring_remaining(mass, stiffness, damping, 0., KEYBOARD_DURATION, RATE)
        .into_iter()
        .map(|left| end + KEYBOARD_TRAVEL * left)
        .collect()
}

#[test]
fn the_native_ride_scrolls_once_on_uikits_ease_in_out_over_the_keyboards_time() {
    use zgx::native_control::{ease_in_out, native_keyboard_scroll};
    // kCAMediaTimingFunctionEaseInEaseOut: symmetric, half way at half time,
    // slow at both ends.
    assert!(ease_in_out(0.).abs() < 1e-9 && (ease_in_out(1.) - 1.).abs() < 1e-9);
    assert!((ease_in_out(0.5) - 0.5).abs() < 1e-6);
    for t in [0.1, 0.25, 0.4] {
        assert!((ease_in_out(t) + ease_in_out(1. - t) - 1.).abs() < 1e-6);
        assert!(ease_in_out(t) < t, "slow from rest at {t}");
    }
    // At a quarter of the time: the curve's value (solved from its control
    // points by hand, 0.1291...).
    assert!(
        (ease_in_out(0.25) - 0.1291).abs() < 0.001,
        "{}",
        ease_in_out(0.25)
    );
    let path = native_keyboard_scroll(100., 300., KEYBOARD_DURATION, RATE);
    assert_eq!(path.len(), (KEYBOARD_DURATION * RATE).ceil() as usize + 1);
    assert_eq!(*path.first().unwrap(), 100.);
    assert_eq!(*path.last().unwrap(), 300.);
    assert!(
        path.windows(2).all(|w| w[1] >= w[0]),
        "one way, no overshoot"
    );
    // The field lands on its own curve: the gap to the keyboard's edge isn't
    // held while they move, as UIKit's isn't.
    let (bottom, end) = (700f32, 538f64);
    let edges = keyboard_edges(end);
    let path = native_keyboard_scroll(0., bottom + ROOM - end as f32, KEYBOARD_DURATION, RATE);
    let gaps: Vec<f32> = edges
        .iter()
        .zip(&path)
        .filter(|(edge, _)| (**edge as f32) < bottom + ROOM)
        .map(|(edge, offset)| *edge as f32 - (bottom - offset))
        .collect();
    let (least, most) = gaps
        .iter()
        .fold((f32::MAX, f32::MIN), |(l, m), g| (l.min(*g), m.max(*g)));
    assert!(most - least > 5., "the gap changes: {least}..{most}");
    assert!(
        (gaps.last().unwrap() - ROOM).abs() < 0.5,
        "and ends at the room"
    );
}

#[test]
fn the_attached_ride_holds_the_fields_gap_to_the_keyboards_edge() {
    use zgx::native_control::keyboard_attached;
    // A field whose bottom is at 700, the page at 0; the keyboard rises to 538.
    let (offset, bottom, end) = (0f32, 700f32, 538f64);
    let start = end + KEYBOARD_TRAVEL;
    let plain = move |edge: f64| offset + (f64::from(bottom + ROOM) - edge).max(0.) as f32;
    let page = keyboard_attached(plain, start, end);
    let edges = keyboard_edges(end);
    let path: Vec<f32> = edges.iter().map(|edge| page(*edge)).collect();
    assert!((path.first().unwrap() - plain(start)).abs() < 0.01);
    assert!((path.last().unwrap() - plain(end)).abs() < 0.5);
    // Once the edge has come a blend's way past the field, the gap holds.
    const BLEND: f32 = 120.;
    for (edge, at) in edges.iter().zip(&path) {
        let field = bottom - (at - offset);
        if (*edge as f32) < bottom + ROOM - BLEND {
            assert!(
                (*edge as f32 - field - ROOM).abs() < 0.01,
                "at edge {edge}: gap {}",
                *edge as f32 - field
            );
        }
        // Never under the keyboard.
        assert!(field + ROOM <= *edge as f32 + 0.01 || *edge as f32 >= bottom + ROOM);
    }
}

#[test]
fn the_showcases_notes_page_switches_between_the_two_rides() {
    use zgx::elastic::KeyboardRide;
    let mut h = Harness::new(|runtime| {
        zgx_script::ScriptHost::new(
            runtime,
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/examples/ios_showcase/app.luau"
            ),
        )
    });
    h.ui.scene.borrow_mut().resize(402., 874.);
    h.frames(2);
    h.host.set_state("tab", zgx_script::Value::Num(2.)).unwrap();
    h.frames(2);
    assert_eq!(h.error(), None);
    let ride = |h: &mut Harness| {
        h.ui.scroll_driver_at(200., 300.)
            .expect("the Notes page's scroll view")
            .keyboard_ride()
    };
    assert_eq!(ride(&mut h), KeyboardRide::Native, "UIKit's, at first");
    h.host
        .set_state("attached", zgx_script::Value::Bool(true))
        .unwrap();
    h.frames(2);
    assert_eq!(h.error(), None);
    assert_eq!(ride(&mut h), KeyboardRide::Attached);
}
