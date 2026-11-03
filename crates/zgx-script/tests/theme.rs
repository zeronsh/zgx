//! Themes from scripts: `"$token"` values, `ui.theme`, app tokens, scoped
//! `theme = …` and theme files that reload in place.
mod common;

use common::Harness;
use std::time::Duration;
use zgx::scene::{Color, NodeKind};
use zgx_script::ScriptHost;

impl Harness {
    fn fill(&mut self, id: &str) -> Color {
        let node = self.find(id).unwrap_or_else(|| panic!("no node `{id}`"));
        match self.ui.scene.borrow().kind(node) {
            NodeKind::Panel { quad, .. } => quad.fill,
            NodeKind::Container(_) => Color(0, 0, 0, 0),
            other => panic!("`{id}` is not a box: {other:?}"),
        }
    }
    fn radius(&mut self, id: &str) -> f32 {
        let node = self.find(id).unwrap();
        match self.ui.scene.borrow().kind(node) {
            NodeKind::Panel { quad, .. } => quad.radius,
            other => panic!("`{id}` is not a box: {other:?}"),
        }
    }
    fn ink(&mut self, id: &str) -> Color {
        let node = self.find(id).unwrap_or_else(|| panic!("no node `{id}`"));
        let scene = self.ui.scene.borrow();
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            if let NodeKind::Text { color, .. } = scene.kind(n) {
                return *color;
            }
            stack.extend(scene.children(n).iter().rev());
        }
        panic!("no text in `{id}`")
    }
}
const DARK_RAISED: Color = Color(35, 46, 64, 255);
const WHITE: Color = Color(255, 255, 255, 255);
const LIGHT_TEXT: Color = Color(0x1a, 0x1f, 0x29, 255);
const DARK_TEXT: Color = Color(229, 237, 247, 255);

#[test]
fn token_values_style_nodes_and_follow_the_theme() {
    let mut h = Harness::inline(
        r##"return ui.column {
            ui.div { id = "card", w = 40, h = 20, bg = "$surface.raised", rounded = "$radius.lg", p = "$space.2" },
            ui.text { "Muted", id = "muted", color = "$text.secondary" },
            ui.text { "Body", id = "body" },
            ui.button { id = "light", w = 20, h = 20, on_click = function() ui.theme.set("zgx-light") end },
        }"##,
    );
    assert_eq!(h.fill("card"), DARK_RAISED);
    assert_eq!(h.radius("card"), 10.);
    assert_eq!(h.ink("muted"), Color(138, 154, 176, 255));
    assert_eq!(h.ink("body"), DARK_TEXT);
    h.ui.scene.borrow_mut().flush();
    h.click("light");
    let report = h.ui.scene.borrow_mut().flush();
    assert_eq!(h.fill("card"), WHITE);
    assert_eq!(h.ink("body"), LIGHT_TEXT);
    assert_eq!(h.ink("muted"), Color(0x5a, 0x64, 0x74, 255));
    assert_eq!(report.layout_nodes, 0, "colours need no layout");
}

#[test]
fn app_tokens_and_luau_themes() {
    let mut h = Harness::inline(
        r##"ui.theme.tokens {
            ["app.card"] = "{surface.raised}",
            app = { badge = "alpha({accent}, 0.5)" },
        }
        ui.theme.register {
            name = "app-sepia",
            type = "light",
            tokens = { ["app.card"] = "#f4ecd8", accent = "#8a5a00" },
        }
        local accent = ui.theme.get("accent")
        return ui.column {
            ui.div { id = "card", w = 10, h = 10, bg = "$app.card" },
            ui.div { id = "badge", w = 10, h = 10, bg = "$app.badge" },
            ui.div { id = "unknown", w = 10, h = 10, bg = "$app.nowhere" },
            ui.text { accent, id = "accent" },
            ui.button { id = "sepia", w = 20, h = 20, on_click = function() ui.theme.set("app-sepia") end },
        }"##,
    );
    assert_eq!(h.fill("card"), DARK_RAISED, "declared in standard terms");
    assert_eq!(h.fill("badge"), Color(0x5e, 0xa5, 0xff, 128));
    assert_eq!(
        h.fill("unknown"),
        Color(0, 0, 0, 0),
        "unknown tokens are clear"
    );
    assert_eq!(h.text("accent"), "#5ea5ffff");
    h.click("sepia");
    assert_eq!(
        h.fill("card"),
        Color(0xf4, 0xec, 0xd8, 255),
        "the theme sets it"
    );
    assert_eq!(
        h.fill("badge"),
        Color(0x8a, 0x5a, 0x00, 128),
        "derived again"
    );
}

#[test]
fn ui_tokens_name_every_token_as_fields() {
    let mut h = Harness::inline(
        r##"ui.theme.tokens { app = { card = "#20242c" } }
        local before = ui.tokens.surface.raised
        -- Declared after the first read: found all the same.
        ui.theme.tokens { app = { late = "#ffffff" } }
        local typo = select(2, pcall(function() return ui.tokens.app.crad end))
        return ui.column {
            ui.div { id = "card", w = 10, h = 10, bg = ui.tokens.app.card },
            ui.div { id = "raised", w = 10, h = 10, bg = before, rounded = ui.tokens.radius.lg },
            ui.div { id = "late", w = 10, h = 10, bg = ui.tokens.app.late },
            ui.text { ui.tokens.accent.value .. " " .. ui.tokens.accent.hover .. " " .. ui.tokens.space[3], id = "names" },
            ui.text { tostring(typo), id = "typo" },
        }"##,
    );
    assert_eq!(h.fill("card"), Color(0x20, 0x24, 0x2c, 255));
    assert_eq!(h.fill("raised"), DARK_RAISED);
    assert_eq!(h.radius("raised"), 10.);
    assert_eq!(h.fill("late"), WHITE);
    assert_eq!(h.text("names"), "$accent $accent.hover $space.3");
    assert!(
        h.text("typo").contains("ui.tokens has no `app.crad`"),
        "{}",
        h.text("typo")
    );
}

#[test]
fn scoped_themes_restyle_their_subtrees() {
    let mut h = Harness::inline(
        r##"local light = ui.state("light", true)
        return ui.column {
            ui.div { id = "outer", w = 10, h = 10, bg = "$surface.raised" },
            ui.column {
                theme = "zgx-light",
                ui.div { id = "card", w = 10, h = 10, bg = "$surface.raised" },
                ui.text { "Inside", id = "inside" },
            },
            ui.div {
                theme = { accent = "#e5484d" },
                ui.div { id = "ring", w = 10, h = 10, bg = "$focus.ring" },
            },
            ui.div {
                theme = function() return if light:get() then "zgx-light" else "zgx-dark" end,
                ui.div { id = "switch", w = 10, h = 10, bg = "$background" },
            },
            ui.button { id = "toggle", w = 20, h = 20, on_click = ui.toggle(light) },
        }"##,
    );
    assert_eq!(h.fill("outer"), DARK_RAISED);
    assert_eq!(h.fill("card"), WHITE);
    assert_eq!(
        h.ink("inside"),
        LIGHT_TEXT,
        "text inherits the scope's body colour"
    );
    assert_eq!(
        h.fill("ring"),
        Color(0xe5, 0x48, 0x4d, 255),
        "derived from the scope's accent"
    );
    assert_eq!(h.fill("switch"), Color(0xf5, 0xf6, 0xf8, 255));
    h.click("toggle");
    assert_eq!(h.fill("switch"), Color(16, 20, 28, 255));
}

#[test]
fn spans_take_tokens() {
    let mut h = Harness::inline(
        r##"return ui.rich {
            id = "rich",
            ui.span { "plain" },
            ui.span { "accent", color = "$accent", bg = "$selection" },
        }"##,
    );
    assert!(h.find("rich").is_some());
    h.ui.scene.borrow_mut().flush();
    h.ui.theme().set(zgx::theme::LIGHT).unwrap();
    h.settle();
    let report = h.ui.scene.borrow_mut().flush();
    assert_eq!(report.layout_nodes, 0, "{report:?}");
}

#[test]
fn theme_files_reload_without_reloading_scripts() {
    let dir = std::env::temp_dir().join(format!("zgx-script-theme-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let theme = dir.join("brand.json");
    let write = |accent: &str, offset: u64| {
        std::fs::write(
            &theme,
            format!(
                r#"{{ "name": "brand", "type": "dark", "tokens": {{ "accent": "{accent}" }} }}"#
            ),
        )
        .unwrap();
        std::fs::File::options()
            .write(true)
            .open(&theme)
            .unwrap()
            .set_modified(std::time::SystemTime::now() + Duration::from_secs(offset))
            .unwrap();
    };
    write("#ff0000", 0);
    std::fs::write(
        dir.join("app.luau"),
        r##"local count = ui.state("count", 0)
        ui.theme.set(ui.theme.load("brand.json"))
        return ui.column {
            ui.div { id = "accent", w = 10, h = 10, bg = "$accent" },
            ui.button { id = "add", w = 20, h = 20, on_click = ui.add(count) },
            ui.text { "{count}", id = "count" },
        }"##,
    )
    .unwrap();
    let path = dir.join("app.luau");
    let mut h = Harness::new(|runtime| ScriptHost::new(runtime, &path));
    assert_eq!(h.fill("accent"), Color(255, 0, 0, 255));
    h.click("add");
    assert_eq!(h.text("count"), "1");
    let before = h.find("accent").unwrap();
    write("#00ff00", 2);
    assert!(h.host.reload_if_changed());
    // Restyled in place: the same node, the state where it was.
    assert_eq!(h.find("accent"), Some(before));
    assert_eq!(h.fill("accent"), Color(0, 255, 0, 255));
    assert_eq!(h.text("count"), "1");
    // A file that stops reading shows its error and keeps the last theme.
    std::fs::write(&theme, "{ nope").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&theme)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(4))
        .unwrap();
    assert!(h.host.reload_if_changed());
    assert!(
        h.host
            .error()
            .get()
            .is_some_and(|e| e.contains("brand.json"))
    );
    assert_eq!(h.fill("accent"), Color(0, 255, 0, 255));
    write("#0000ff", 6);
    assert!(h.host.reload_if_changed());
    assert_eq!(h.host.error().get(), None);
    assert_eq!(h.fill("accent"), Color(0, 0, 255, 255));
    // Theme files stay inside the script's directory.
    std::fs::write(
        dir.join("app.luau"),
        r##"ui.theme.load("../elsewhere.json") return ui.div {}"##,
    )
    .unwrap();
    assert!(h.host.reload().is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn defaults_survive_reloads_but_not_a_choice() {
    let h = Harness::inline(
        r##"ui.theme.default("zgx-light")
        return ui.div {}"##,
    );
    assert_eq!(&*h.ui.theme().current(), "zgx-light");
    h.ui.theme().set(zgx::theme::DARK).unwrap();
    h.host.reload().unwrap();
    assert_eq!(
        &*h.ui.theme().current(),
        "zgx-dark",
        "the app's choice stands"
    );
}

#[test]
fn the_current_theme_is_state() {
    let mut h = Harness::inline(
        r##"return ui.column {
            ui.text { function() return ui.theme.current:get() .. " " .. ui.theme.kind() end, id = "current" },
            ui.button { id = "light", w = 20, h = 20, on_click = function() ui.theme.current:set("zgx-light") end },
            ui.button { id = "bad", w = 20, h = 20, on_click = function() ui.theme.current:set("no-such") end },
        }"##,
    );
    assert_eq!(h.text("current"), "zgx-dark dark");
    h.ui.theme().set("zgx-light").unwrap();
    h.settle();
    assert_eq!(h.text("current"), "zgx-light light", "Rust's switch shows");
    h.ui.theme().set("zgx-dark").unwrap();
    h.click("light");
    assert_eq!(
        &*h.ui.theme().current(),
        "zgx-light",
        "the script's applies"
    );
    h.click("bad");
    assert_eq!(&*h.ui.theme().current(), "zgx-light");
    assert_eq!(h.text("current"), "zgx-light light");
}

/// Every fill and text colour in the scene.
fn colors(ui: &zgx::widgets::Ui) -> Vec<Color> {
    let scene = ui.scene.borrow();
    let mut out = Vec::new();
    let mut stack = vec![scene.root()];
    while let Some(id) = stack.pop() {
        match scene.kind(id) {
            NodeKind::Panel { quad, .. } => out.push(quad.fill),
            NodeKind::Text { color, .. } => out.push(*color),
            _ => {}
        }
        stack.extend(scene.children(id).iter().copied());
    }
    out
}

#[test]
fn the_notes_example_keeps_its_look_and_takes_any_theme() {
    let app = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/notes/app.luau");
    let h = Harness::new(|runtime| ScriptHost::new(runtime, app));
    assert_eq!(h.host.error().get(), None);
    h.ui.prepare_frame();
    assert_eq!(&*h.ui.theme().current(), "notes-glass");
    let glass = colors(&h.ui);
    for (color, what) in [
        (Color(0x12, 0x14, 0x16, 0x6b), "the window's glass"),
        (Color(0x1b, 0x1c, 0x20, 0xd9), "the editor"),
        (Color(0x1b, 0x1c, 0x20, 0xf0), "the preview"),
        (Color(0xff, 0xff, 0xff, 0x10), "the search field"),
        (Color(0xff, 0xff, 0xff, 0xeb), "text"),
        (Color(0xf5, 0xb7, 0x0a, 0x3d), "the selected note"),
    ] {
        assert!(glass.contains(&color), "{what}: {color:?} not in {glass:?}");
    }
    h.ui.theme().set(zgx::theme::LIGHT).unwrap();
    h.settle();
    h.ui.prepare_frame();
    let light = colors(&h.ui);
    assert!(light.contains(&WHITE), "the editor on the light base");
    assert!(light.contains(&LIGHT_TEXT));
    assert!(!light.contains(&Color(0xff, 0xff, 0xff, 0xeb)));
    assert_eq!(h.host.error().get(), None);
}

#[test]
fn ui_prefers_follows_the_systems_display_settings() {
    use zgx::theme::SystemPreferences;
    let mut h = Harness::inline(
        r##"return ui.column {
            ui.text { function()
                local p = ui.prefers
                return `{p.reduced_motion:get()} {p.more_contrast:get()} {p.reduced_transparency:get()}`
            end, id = "p" } }"##,
    );
    h.frames(2);
    assert_eq!(h.text("p"), "false false false");
    h.ui.theme().set_system_preferences(SystemPreferences {
        reduced_motion: true,
        increased_contrast: false,
        reduced_transparency: true,
    });
    h.frames(1);
    assert_eq!(h.text("p"), "true false true", "as the system changes");
}

/// A box's corner radii (top left, top right, bottom right, bottom left)
/// and border widths (top, right, bottom, left) as painted.
fn quad_of(h: &mut Harness, id: &str) -> ([f32; 4], [f32; 4]) {
    let node = h.find(id).unwrap_or_else(|| panic!("no node `{id}`"));
    match h.ui.scene.borrow().kind(node) {
        NodeKind::Panel { quad, .. } | NodeKind::Quad(quad) => {
            let d = quad.decoration.as_ref();
            let corners = d.and_then(|d| d.corners).map_or([quad.radius; 4], |c| {
                [c.top_left, c.top_right, c.bottom_right, c.bottom_left]
            });
            let borders = d
                .and_then(|d| d.border_widths)
                .map_or([quad.border_width; 4], |b| {
                    [b.top, b.right, b.bottom, b.left]
                });
            (corners, borders)
        }
        other => panic!("`{id}` is not a box: {other:?}"),
    }
}

const SHORTHANDS: &str = r##"
ui.theme.tokens { app = { r = 24, s = 10, b = 3 } }
local function box(p) p.w, p.h, p.bg = 160, 80, "#3a3a44"; return ui.div(p) end
local function holder(p) p.bg = "#3a3a44"; p[1] = ui.div { id = p.id .. "-in", w = 20, h = 20 }; return ui.column(p) end
return ui.column { gap = 4,
    box { id = "r-token", rounded = "$app.r", rounded_tl = 0, rounded_tr = 0 },
    box { id = "r-num", rounded = 24, rounded_tl = 0, rounded_tr = 0 },
    box { id = "r-token-corners", rounded = "$app.r", corners = { 1, 2, 3, 4 } },
    box { id = "b-token", border = "$app.b", border_color = "#ffffff", border_sides = { 0, 1, 0, 1 } },
    box { id = "b-num", border = 3, border_color = "#ffffff", border_sides = { 0, 1, 0, 1 } },
    holder { id = "p-token", p = "$app.s", pt = 0, pl = 2 },
    holder { id = "p-num", p = 10, pt = 0, pl = 2 },
    ui.row { id = "m-token-row", holder { id = "m-token", m = "$app.s", mt = 0, ml = 2 } },
    ui.row { id = "m-num-row", holder { id = "m-num", m = 10, mt = 0, ml = 2 } },
}"##;

/// `rounded = "$app.r", rounded_tl = 0, rounded_tr = 0` rounds the
/// bottom corners by the token, as `rounded = 24` does: the token resolves
/// first and the single corners lie over it. `corners` replaces it whole.
#[test]
fn a_token_radius_takes_its_single_corners_as_a_number_does() {
    let mut h = Harness::inline(SHORTHANDS);
    assert_eq!(h.error(), None);
    assert_eq!(quad_of(&mut h, "r-num").0, [0., 0., 24., 24.]);
    assert_eq!(quad_of(&mut h, "r-token").0, [0., 0., 24., 24.]);
    assert_eq!(quad_of(&mut h, "r-token-corners").0, [1., 2., 3., 4.]);
}

/// `border = "$app.b", border_sides = { … }` gives the sides, as
/// `border = 3` does.
#[test]
fn a_token_border_takes_its_sides_as_a_number_does() {
    let mut h = Harness::inline(SHORTHANDS);
    assert_eq!(quad_of(&mut h, "b-num").1, [0., 1., 0., 1.]);
    assert_eq!(quad_of(&mut h, "b-token").1, [0., 1., 0., 1.]);
}

/// Where a box's first child lies inside it: its padding at the left and
/// top.
fn inner(h: &mut Harness, id: &str) -> (f32, f32) {
    let (outer, inner) = (h.bounds(id), h.bounds(&format!("{id}-in")));
    (inner.x - outer.x, inner.y - outer.y)
}

/// `p = "$app.s", pt = 0, pl = 2` pads the top 0 and the left 2, as
/// `p = 10` does.
#[test]
fn a_token_padding_takes_its_sides_as_a_number_does() {
    let mut h = Harness::inline(SHORTHANDS);
    assert_eq!(inner(&mut h, "p-num"), (2., 0.));
    assert_eq!(inner(&mut h, "p-token"), (2., 0.));
}

/// `m = "$app.s", mt = 0, ml = 2` sets the top margin 0 and the left 2,
/// as `m = 10` does.
#[test]
fn a_token_margin_takes_its_sides_as_a_number_does() {
    let mut h = Harness::inline(SHORTHANDS);
    let outer = |h: &mut Harness, id: &str| {
        let (row, b) = (h.bounds(&format!("{id}-row")), h.bounds(id));
        (b.x - row.x, b.y - row.y)
    };
    assert_eq!(outer(&mut h, "m-num"), (2., 0.));
    assert_eq!(outer(&mut h, "m-token"), (2., 0.));
}

/// Text taking `$text.primary` or `$text.secondary` from the row around it
/// follows every theme switch, not only the first: switched by the script
/// (light, dark, light, dark) and by the system while the app follows it.
/// (After the first, a text's paint-only style run resolved its inherited
/// size and family again but not its colour, which stayed the first
/// switch's.)
#[test]
fn inherited_token_text_colours_follow_every_theme_switch() {
    use zgx::theme::{Appearance, tok};
    let mut h = Harness::inline(
        r##"ui.theme.current:set("zgx-dark")
local function switch(id, name)
    return ui.button { id = id, on_click = function() ui.theme.current:set(name) end, color = "#ff0000ff", "x" }
end
return ui.column {
    switch("light", "zgx-light"), switch("dark", "zgx-dark"),
    ui.row { id = "primary", color = "$text.primary", "Continue" },
    ui.row { id = "secondary", color = "$text.secondary", "Later" },
}"##,
    );
    h.frame();
    // The theme's own colours, and which theme it is.
    let check = |h: &mut Harness, when: &str, dark: bool| {
        let scope = h.ui.theme_scope();
        let primary = scope.color(tok::TEXT_PRIMARY);
        let secondary = scope.color(tok::TEXT_SECONDARY);
        let theme = if dark { DARK_TEXT } else { LIGHT_TEXT };
        assert_eq!(primary, theme, "{when}: the theme");
        assert_eq!(h.ink("primary"), primary, "{when}: text.primary");
        assert_eq!(h.ink("secondary"), secondary, "{when}: text.secondary");
    };
    check(&mut h, "at first", true);
    for (i, id) in ["light", "dark", "light", "dark"].into_iter().enumerate() {
        h.click(id);
        h.frame();
        check(&mut h, &format!("switch {i} to {id}"), id == "dark");
    }
    // Following the system (dark, light), its appearance changing again
    // and again.
    h.ui.theme().follow_system(Some(("zgx-dark", "zgx-light")));
    let appearances = [
        Appearance::Light,
        Appearance::Dark,
        Appearance::Light,
        Appearance::Dark,
    ];
    for (i, appearance) in appearances.into_iter().enumerate() {
        h.ui.theme().set_system_appearance(appearance);
        h.frame();
        check(
            &mut h,
            &format!("system change {i}"),
            appearance == Appearance::Dark,
        );
    }
}
