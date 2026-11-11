//! A world's sky (`World::set_sky`, feature `media`): a stock scene draw
//! (`render::SkyDraw`) behind the meshes, the world kept drawing only while
//! its clouds drift.
use super::{Settle, World};
use crate::{Sky, render::SceneDraw, render::SkyDraw};
use std::{cell::RefCell, rc::Rc};

impl Settle {
    /// Its sky's clouds drift.
    pub(crate) const SKY: u32 = super::Reason::Sky as u32;
}

impl World {
    /// Its sky (`None`, the default: the background colour behind the
    /// meshes), lit by its first directional light as the sun: the air's
    /// light where no mesh is, from the next frame on, in its lights' units
    /// (as `Sky::sun_light` and `Sky::environment` give them). A sky whose
    /// clouds drift keeps the world drawing; a still one lets it rest.
    pub fn set_sky(&self, sky: Option<Sky>) {
        let mut added = None;
        let mut removed = None;
        self.change_view(|inner| {
            if inner.sky.as_ref().map(|(s, _)| s) == sky.as_ref() {
                return false;
            }
            match (sky, &inner.sky) {
                (Some(sky), Some((_, draw))) => {
                    let (uniform, speed) = sky.uniform();
                    draw.borrow_mut().set(uniform, speed);
                    let draw = draw.clone();
                    inner.sky = Some((sky, draw));
                }
                (Some(sky), None) => {
                    let (uniform, speed) = sky.uniform();
                    let draw = Rc::new(RefCell::new(SkyDraw::new(uniform, speed)));
                    inner.sky = Some((sky, draw.clone()));
                    added = Some(draw);
                }
                (None, _) => removed = inner.sky.take().map(|(_, draw)| draw),
            }
            true
        });
        if let Some(draw) = added {
            self.add_scene_draw(draw);
        }
        if let Some(draw) = removed {
            let draw: Rc<RefCell<dyn SceneDraw>> = draw;
            self.remove_scene_draw(&draw);
        }
        let inner = self.inner.borrow();
        let drifting = inner
            .sky
            .as_ref()
            .is_some_and(|(_, draw)| draw.borrow().speed != 0.);
        inner.unsettle(Settle::SKY, drifting);
    }
    /// How many draw into its scene with the meshes, for the tests.
    #[cfg(test)]
    pub(crate) fn scene_draws(&self) -> usize {
        self.inner.borrow().scene_draws.len()
    }
    /// Its sky (`World::set_sky`).
    pub fn sky(&self) -> Option<Sky> {
        self.inner.borrow().sky.as_ref().map(|(sky, _)| *sky)
    }
}
