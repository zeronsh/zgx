//! Handling of `ext-background-effect-v1`: the compositor blurs what lies
//! behind a surface's blur region (Hyprland, KWin and others; the standard
//! successor to `org_kde_kwin_blur`).

use sctk::compositor::{CompositorState, Region};
use sctk::reexports::client::globals::{BindError, GlobalList};
use sctk::reexports::client::protocol::wl_surface::WlSurface;
use sctk::reexports::client::{delegate_dispatch, Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::{
    Event as ManagerEvent, ExtBackgroundEffectManagerV1,
};
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1;

use sctk::globals::GlobalData;

use crate::platform_impl::wayland::state::WinitState;

/// Background effect manager.
#[derive(Debug, Clone)]
pub struct BackgroundEffectManager {
    manager: ExtBackgroundEffectManagerV1,
}

impl BackgroundEffectManager {
    pub fn new(
        globals: &GlobalList,
        queue_handle: &QueueHandle<WinitState>,
    ) -> Result<Self, BindError> {
        let manager = globals.bind(queue_handle, 1..=1, GlobalData)?;
        Ok(Self { manager })
    }

    /// Blur the whole of what lies behind `surface`, from its next commit.
    pub fn blur(
        &self,
        surface: &WlSurface,
        compositor: &CompositorState,
        queue_handle: &QueueHandle<WinitState>,
    ) -> Option<ExtBackgroundEffectSurfaceV1> {
        let region = Region::new(compositor).ok()?;
        region.add(0, 0, i32::MAX, i32::MAX);
        let effect = self.manager.get_background_effect(surface, queue_handle, ());
        effect.set_blur_region(Some(region.wl_region()));
        Some(effect)
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, GlobalData, WinitState> for BackgroundEffectManager {
    fn event(
        _: &mut WinitState,
        _: &ExtBackgroundEffectManagerV1,
        _: ManagerEvent,
        _: &GlobalData,
        _: &Connection,
        _: &QueueHandle<WinitState>,
    ) {
        // (`capabilities`: what the compositor can apply now. A blur region
        // is asked for either way; the compositor applies it once it can.)
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, (), WinitState> for BackgroundEffectManager {
    fn event(
        _: &mut WinitState,
        _: &ExtBackgroundEffectSurfaceV1,
        _: <ExtBackgroundEffectSurfaceV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<WinitState>,
    ) {
        unreachable!("no events defined for ext_background_effect_surface_v1");
    }
}

delegate_dispatch!(WinitState: [ExtBackgroundEffectManagerV1: GlobalData] => BackgroundEffectManager);
delegate_dispatch!(WinitState: [ExtBackgroundEffectSurfaceV1: ()] => BackgroundEffectManager);
