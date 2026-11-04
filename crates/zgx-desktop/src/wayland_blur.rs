//! Whether the Wayland compositor blurs behind a window that asks
//! (`WindowCapabilities::blur_behind`): whether it has a protocol winit's
//! `set_blur` asks through. Read once, on a connection of its own, so an app
//! on a winit from crates.io (which has no native popups, and no client of
//! zgx's on its display) is told too. KWin has `org_kde_kwin_blur_manager`
//! while its blur effect is on, as a launch finds it; Hyprland
//! `ext-background-effect-v1`, which only the repository's winit asks through.
//! GNOME, sway and Weston have neither, and glass there shows what is behind
//! it sharp.
use std::sync::OnceLock;
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    protocol::wl_registry::{self, WlRegistry},
};

#[derive(Default)]
struct Globals(Vec<String>);
impl Dispatch<WlRegistry, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { interface, .. } = event {
            globals.0.push(interface);
        }
    }
}

/// Whether `globals` hold a blur protocol the app's winit asks through (with
/// `patched`, the repository's).
fn asks_through(globals: &[String], patched: bool) -> bool {
    globals.iter().any(|global| {
        global == "org_kde_kwin_blur_manager"
            || (patched && global == "ext_background_effect_manager_v1")
    })
}

/// Whether this session's Wayland compositor blurs behind a window that asks.
pub(crate) fn offered() -> bool {
    static OFFERED: OnceLock<bool> = OnceLock::new();
    *OFFERED.get_or_init(|| {
        let read = || -> Option<Vec<String>> {
            let connection = Connection::connect_to_env().ok()?;
            let mut queue = connection.new_event_queue();
            connection.display().get_registry(&queue.handle(), ());
            let mut globals = Globals::default();
            queue.roundtrip(&mut globals).ok()?;
            Some(globals.0)
        };
        read().is_some_and(|globals| asks_through(&globals, cfg!(zgx_winit_patched)))
    })
}

#[cfg(test)]
mod tests {
    use super::asks_through;

    #[test]
    fn a_compositor_blurs_if_it_has_a_protocol_winit_asks_through() {
        let globals = |names: &[&str]| {
            names
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        };
        let kwin = globals(&["wl_compositor", "org_kde_kwin_blur_manager"]);
        let hyprland = globals(&["wl_compositor", "ext_background_effect_manager_v1"]);
        let gnome = globals(&["wl_compositor", "xdg_wm_base"]);
        assert!(asks_through(&kwin, false) && asks_through(&kwin, true));
        assert!(asks_through(&hyprland, true));
        // Upstream winit asks through KWin's alone.
        assert!(!asks_through(&hyprland, false));
        assert!(!asks_through(&gnome, true) && !asks_through(&gnome, false));
    }
}
