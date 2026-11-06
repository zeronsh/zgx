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

/// What a hosted tab bar shows and whom it tells.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub struct TabsState {
    /// Each tab's title and system symbol.
    pub items: Vec<(String, String)>,
    /// The chosen tab, as last read.
    pub selected: std::cell::Cell<usize>,
    /// The person chose a tab.
    pub changed: Box<dyn Fn(usize)>,
}

/// The system's tab bar: its glass, and the lens that morphs between tabs,
/// which nothing drawn matches. `items` are each tab's title and SF Symbol.
/// `selected` runs again when state it reads changes; `set` hears of each
/// choice. Each tab is one element to assistive technology, the bar's own hidden.
/// Elsewhere, a row of buttons. Give it the bar's height, 83 points.
pub fn native_tabs(
    items: Vec<(String, String)>,
    selected: impl FnMut() -> usize + 'static,
    set: impl Fn(usize) + 'static,
) -> View {
    component(move |cx| {
        let mut selected = selected;
        let state = Rc::new(TabsState {
            selected: std::cell::Cell::new(selected()),
            changed: Box::new(set),
            items,
        });
        let current = cx.state(state.selected.get());
        {
            let (state, current) = (state.clone(), current.clone());
            let effect = cx.runtime().effect(move || {
                let next = selected();
                state.selected.set(next);
                if current.with_untracked(|now| *now != next) {
                    current.set(next);
                }
                crate::native_view::wake();
            });
            cx.retain(effect);
        }
        let mut bar = row().w_full().h_full().role(Role::TabList);
        for (index, (title, _)) in state.items.iter().enumerate() {
            let (chosen, choose, state) = (current.clone(), state.clone(), state.clone());
            let _ = choose;
            bar = bar.child(
                div()
                    .flex_grow(1.)
                    .h_full()
                    .role(Role::Tab)
                    .accessible_label(title.clone())
                    .selected_when(move || chosen.get() == index)
                    .on_click(move || (state.changed)(index)),
            );
        }
        #[cfg(target_os = "ios")]
        {
            let natives = cx.try_service::<crate::native_view::NativeViews>();
            bar.on_mount(move |handle| {
                let Some(natives) = natives else { return };
                if natives.supported()
                    && let Some(scene) = handle.scene()
                    && let Ok(mut scene) = scene.try_borrow_mut()
                {
                    scene.set_platform_painted(handle.node(), true);
                }
                handle.retain(natives.register(
                    handle.node(),
                    crate::native_view::NativeContent::Tabs(state),
                ));
            })
        }
        #[cfg(not(target_os = "ios"))]
        {
            let _ = &state;
            bar
        }
    })
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
