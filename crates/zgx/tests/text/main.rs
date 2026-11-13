//! Text, selection, semantics and themes. One test binary for the area (see AGENTS.md):
//! a test file goes in the group for its area, listed here.
#[path = "../accessible_label.rs"]
mod accessible_label;
#[path = "../composition_rich_text.rs"]
mod composition_rich_text;
#[path = "../find_in_page.rs"]
mod find_in_page;
#[path = "../i18n.rs"]
mod i18n;
#[path = "../inspect.rs"]
mod inspect;
#[cfg(feature = "i18n-intl")]
#[path = "../intl_parity.rs"]
mod intl_parity;
#[path = "../links.rs"]
mod links;
#[path = "../selectable_text.rs"]
mod selectable_text;
#[path = "../system_preferences.rs"]
mod system_preferences;
#[path = "../text_direction.rs"]
mod text_direction;
#[path = "../text_hyphens.rs"]
mod text_hyphens;
#[path = "../text_lang.rs"]
mod text_lang;
#[path = "../text_range_area.rs"]
mod text_range_area;
#[path = "../text_wrap.rs"]
mod text_wrap;
#[path = "../theme.rs"]
mod theme;
#[path = "../zoomed_text_geometry.rs"]
mod zoomed_text_geometry;
