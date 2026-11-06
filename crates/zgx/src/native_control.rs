//! Controls that are the system's own where exactness needs them: iOS's
//! `UISwitch`, whose translucent, morphing track nothing drawn matches.
//!
//! On iOS the control is a native view between the planes, clipped by what
//! clips its node, and touches go to it. Its node is the one accessibility
//! element, so assistive technology hears zgx's tree alone, and activating
//! that element toggles the control. Elsewhere it is drawn.
use crate::compose::prelude::*;
use crate::semantics::Role;
use std::rc::Rc;

/// What a hosted toggle shows and whom it tells.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub struct ToggleState {
    /// Whether it is on, as last read.
    pub on: std::cell::Cell<bool>,
    /// The person changed it.
    pub changed: Box<dyn Fn(bool)>,
}

/// An on and off control named `label`: `is_on` runs again when state it
/// reads changes, and `set` hears of each change. Put it in a virtualized
/// list when there are many: each costs about 75 KB while mounted.
pub fn native_toggle(
    label: impl Into<String>,
    is_on: impl FnMut() -> bool + 'static,
    set: impl Fn(bool) + 'static,
) -> View {
    let label = label.into();
    component(move |cx| {
        let mut is_on = is_on;
        let state = Rc::new(ToggleState {
            on: std::cell::Cell::new(is_on()),
            changed: Box::new(set),
        });
        let on = cx.state(state.on.get());
        {
            let (state, on) = (state.clone(), on.clone());
            let effect = cx.runtime().effect(move || {
                let next = is_on();
                state.on.set(next);
                if on.with_untracked(|now| *now != next) {
                    on.set(next);
                }
                crate::native_view::wake();
            });
            cx.retain(effect);
        }
        let toggle = {
            let state = state.clone();
            move || (state.changed)(!state.on.get())
        };
        let checked = on.clone();
        let view = div()
            .w(63.)
            .h(28.)
            .flex_shrink(0.)
            .rounded(14.)
            .role(Role::Switch)
            .accessible_label(label)
            .checked_when(move || checked.get())
            .on_click(toggle);
        #[cfg(target_os = "ios")]
        {
            let natives = cx.try_service::<crate::native_view::NativeViews>();
            view.on_mount(move |handle| {
                let Some(natives) = natives else { return };
                if natives.supported()
                    && let Some(scene) = handle.scene()
                    && let Ok(mut scene) = scene.try_borrow_mut()
                {
                    scene.set_platform_painted(handle.node(), true);
                }
                handle.retain(natives.register(
                    handle.node(),
                    crate::native_view::NativeContent::Toggle(state),
                ));
            })
        }
        #[cfg(not(target_os = "ios"))]
        {
            let _ = &state;
            let (track, thumb) = (on.clone(), on);
            view.reactive_style(move || {
                Styles::new().bg(if track.get() {
                    crate::style::rgb(0x34c759)
                } else {
                    crate::style::rgb(0xe5e5ea)
                })
            })
            .child(
                div()
                    .absolute()
                    .top(2.)
                    .w(39.)
                    .h(24.)
                    .rounded(12.)
                    .bg(crate::style::rgb(0xffffff))
                    .reactive_style(move || Styles::new().left(if thumb.get() { 22. } else { 2. })),
            )
        }
    })
}
