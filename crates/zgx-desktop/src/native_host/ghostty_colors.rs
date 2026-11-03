//! The theme's terminal colours (`zgx::terminal::TerminalColors`) as Ghostty
//! configuration, loaded over the user's own: a colour the theme leaves
//! unset writes no line, so the user's stays.
use std::fmt::Write;
use zgx::{scene::Color, terminal::TerminalColors};

/// `colors` as Ghostty config lines (`foreground = #rrggbb`, `palette =
/// 1=#rrggbb`); empty where the theme sets none. Ghostty's colours are
/// opaque: alpha is dropped.
pub(crate) fn config(colors: &TerminalColors) -> String {
    let mut out = String::new();
    let mut line = |key: &str, color: Option<Color>| {
        if let Some(Color(r, g, b, _)) = color {
            let _ = writeln!(out, "{key} = #{r:02x}{g:02x}{b:02x}");
        }
    };
    line("foreground", colors.foreground);
    line("background", colors.background);
    line("cursor-color", colors.cursor);
    line("cursor-text", colors.cursor_text);
    line("selection-background", colors.selection_background);
    line("selection-foreground", colors.selection_foreground);
    for (index, color) in colors.ansi.iter().enumerate() {
        if let Some(Color(r, g, b, _)) = color {
            let _ = writeln!(out, "palette = {index}=#{r:02x}{g:02x}{b:02x}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::config;
    use zgx::{
        reactive::Runtime,
        terminal::TerminalColors,
        theme::{DARK, LIGHT, Theme, Themes},
    };

    #[test]
    fn an_unset_colour_writes_nothing_so_the_users_stays() {
        assert_eq!(config(&TerminalColors::default()), "");
        // zgx's own themes set no terminal colours.
        let runtime = Runtime::new();
        let themes = Themes::of(&runtime);
        for name in [DARK, LIGHT] {
            themes.set(name).unwrap();
            assert_eq!(config(&TerminalColors::of(themes.root())), "", "{name}");
        }
    }

    #[test]
    fn the_config_follows_each_theme_switch() {
        let runtime = Runtime::new();
        let themes = Themes::of(&runtime);
        themes.register(
            Theme::from_json(
                r##"{ "name": "test-paper", "type": "light", "tokens": {
                    "terminal.foreground": "#202124",
                    "terminal.background": "#fafafa",
                    "terminal.cursor": "#1a73e8",
                    "terminal.cursor_text": "#ffffff",
                    "terminal.selection_background": "#1a73e840",
                    "terminal.selection_foreground": "{terminal.foreground}",
                    "terminal.ansi.1": "#b00020",
                    "terminal.ansi.15": "#000000" } }"##,
            )
            .unwrap(),
        );
        themes.register(
            Theme::from_json(
                r##"{ "name": "test-ink", "type": "dark", "tokens": {
                    "terminal.foreground": "#e8eaed", "terminal.ansi.1": "#f28b82" } }"##,
            )
            .unwrap(),
        );
        let paper = "\
foreground = #202124
background = #fafafa
cursor-color = #1a73e8
cursor-text = #ffffff
selection-background = #1a73e8
selection-foreground = #202124
palette = 1=#b00020
palette = 15=#000000
";
        // Only what the theme sets: the rest stays the user's.
        let ink = "foreground = #e8eaed\npalette = 1=#f28b82\n";
        let colors = || config(&TerminalColors::of(themes.root()));
        for _ in 0..3 {
            themes.set("test-paper").unwrap();
            assert_eq!(colors(), paper);
            themes.set("test-ink").unwrap();
            assert_eq!(colors(), ink);
            themes.set(DARK).unwrap();
            assert_eq!(colors(), "");
        }
    }
}
