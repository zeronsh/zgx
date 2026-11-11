//! Registering what draws into the scene with the meshes
//! (`render::SceneDraw`).
use super::World;
use crate::render::SceneDraw;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg_attr(not(test), allow(dead_code))] // Emitters, decals and debug draws will.
impl World {
    /// `draw` draws into its scene with the meshes from the next frame on,
    /// after those added before it.
    pub(crate) fn add_scene_draw(&self, draw: Rc<RefCell<dyn SceneDraw>>) {
        // Ids no other registration has had, in any world.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        self.change_view(|inner| {
            inner.scene_draws.push((id, draw));
            true
        });
    }
    /// `draw` draws no more.
    pub(crate) fn remove_scene_draw(&self, draw: &Rc<RefCell<dyn SceneDraw>>) {
        self.change_view(|inner| {
            let before = inner.scene_draws.len();
            inner.scene_draws.retain(|(_, d)| !Rc::ptr_eq(d, draw));
            inner.scene_draws.len() != before
        });
    }
}
