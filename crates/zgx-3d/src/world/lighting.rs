//! The lighting milestone's settings on a world (design-3d-aaa.md, 6.3):
//! clustered lights and point and spot lights in physical units. Behind the
//! `lighting` feature.
use super::{ByIndex, Inner, Light, LightId, Settle, Spot, World};
use crate::math::Vec3;

impl Settle {
    /// The sun's cascades have one still to follow the camera.
    pub(crate) const CASCADES: u32 = 1 << 0;
}

/// A world's lighting settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Lighting {
    /// Point and spot lights shaded through clusters whatever their count
    /// (`World::set_clustered_lights`).
    pub(crate) clustered: bool,
    /// Point lights' radii (`PointLight::radius`), by light index, with the
    /// generation they were set for: most have none.
    radii: ByIndex<(u32, f32)>,
    /// The sun's cascades (`World::set_cascades`), while it casts shadows.
    pub(crate) cascades: Option<Cascades>,
    /// Lights and listings the last binning left out past its caps
    /// (`World::lights_left_out`).
    pub(crate) left_out: std::cell::Cell<u32>,
}
impl Lighting {
    /// Whether any light was given a radius (gone or not).
    pub(crate) fn any_radii(&self) -> bool {
        !self.radii.is_empty()
    }
    /// Light `index` (of `generation`)'s radius: 0 for a point.
    pub(crate) fn radius(&self, index: usize, generation: u32) -> f32 {
        match self.radii.get(&(index as u32)) {
            Some(&(set_for, radius)) if set_for == generation => radius,
            _ => 0.,
        }
    }
}

impl Inner {
    /// Whether a light still in the world has a radius.
    pub(crate) fn has_radii(&self) -> bool {
        let lighting = &self.lighting;
        !lighting.radii.is_empty()
            && lighting.radii.iter().any(|(&index, &(generation, _))| {
                self.lights
                    .get(index as usize)
                    .is_some_and(|slot| slot.generation == generation && slot.value.is_some())
            })
    }
}

/// The sun's shadow cut into cascades (`World::set_cascades`): the view
/// cut by depth into `count` slices up to `distance` in front of the
/// camera, each shadowed by a map of its own `size` texels square, the
/// nearer the sharper.
///
/// Made by `Cascades::new` and its setters; fields set by hand are brought
/// within the setters' bounds by `World::set_cascades`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Cascades {
    /// How many: 1 to 4.
    pub count: u32,
    /// How far in front of the camera they reach (and the camera's far
    /// plane at most); beyond, nothing is in shadow.
    pub distance: f32,
    /// Each one's map's side, in texels.
    pub size: u32,
    /// Where they split: 0 evenly, 1 each a constant ratio deeper than the
    /// one before (the near ones smallest); between, a mix.
    pub split: f32,
}
impl Cascades {
    /// `count` cascades (1 to 4) of 2048 texels reaching 100 m, split 0.75
    /// of the way to logarithmically.
    pub fn new(count: u32) -> Self {
        Self {
            count: count.clamp(1, 4),
            distance: 100.,
            size: 2048,
            split: 0.75,
        }
    }
    /// How far in front of the camera they reach.
    pub fn distance(self, distance: f32) -> Self {
        Self {
            distance: distance.max(1e-2),
            ..self
        }
    }
    /// Each one's map's side, in texels (64 to 4096).
    pub fn size(self, size: u32) -> Self {
        Self {
            size: size.clamp(64, 4096),
            ..self
        }
    }
    /// Where they split, 0 (evenly) to 1 (logarithmically).
    pub fn split(self, split: f32) -> Self {
        Self {
            split: if split.is_finite() {
                split.clamp(0., 1.)
            } else {
                0.75
            },
            ..self
        }
    }
    /// It within its setters' bounds, whatever its fields were set to: 1 to
    /// 4 cascades, maps of 64 to 4096 texels, a finite distance over 0, a
    /// split of 0 to 1.
    pub(crate) fn sanitized(self) -> Self {
        let distance = if self.distance.is_finite() && self.distance > 0. {
            self.distance
        } else {
            100.
        };
        Self {
            count: self.count.clamp(1, 4),
            ..self
        }
        .distance(distance)
        .size(self.size)
        .split(self.split)
    }
}

/// A point or spot light in physical units (`World::add_point_light`):
/// `Light::point(position).lumens(800.)`, `Light::spot(position,
/// direction).lumens(4000.).cone(0.3, 0.5)`.
///
/// Its intensity is in candela, as `Light::Point`'s `intensity` is (the
/// sun's in lux: 3 is a bright day for these materials, at exposure 0).
/// Given in lumens, a point light's is `lumens / 4π` candela, and a spot's
/// the same whatever its cone (as Bevy and Filament take a spot's lumens):
/// narrowing a spot's cone keeps how bright it is where it shines.
///
/// Made by `Light::point` or `Light::spot` and their setters; values set
/// by hand are checked as `World::add_light` checks a light's.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PointLight {
    /// Where it is.
    pub position: Vec3,
    /// Its colour, linear.
    pub color: [f32; 3],
    /// How bright, in candela.
    pub candela: f32,
    /// How far it reaches: it falls smoothly to nothing there.
    pub range: f32,
    /// How big it is, a sphere of this radius: falling off from its
    /// surface rather than its centre, so it lights what it nearly touches
    /// without a burning point, and its highlights on shiny surfaces are
    /// as wide as it looks. 0: a point.
    pub radius: f32,
    /// A spot's cone, if it is one.
    pub spot: Option<Spot>,
}

impl Light {
    /// A point light at `position`: white, 1 candela, reaching 10 m.
    pub fn point(position: impl Into<Vec3>) -> PointLight {
        PointLight {
            position: position.into(),
            color: [1.; 3],
            candela: 1.,
            range: 10.,
            radius: 0.,
            spot: None,
        }
    }
    /// A spot light at `position` shining along `direction`: as
    /// `Light::point`, fully within 30° of its direction and fading to
    /// nothing at 45° (`PointLight::cone`).
    pub fn spot(position: impl Into<Vec3>, direction: impl Into<Vec3>) -> PointLight {
        PointLight {
            spot: Some(Spot::new(
                direction,
                30_f32.to_radians(),
                45_f32.to_radians(),
            )),
            ..Self::point(position)
        }
    }
}

impl PointLight {
    /// Its colour, linear.
    pub fn color(self, color: [f32; 3]) -> Self {
        Self { color, ..self }
    }
    /// How bright, in candela (lumens per steradian).
    pub fn candela(self, candela: f32) -> Self {
        Self {
            candela: candela.max(0.),
            ..self
        }
    }
    /// How bright, as the lumens it gives off in all: `lumens / 4π`
    /// candela, a spot's too.
    pub fn lumens(self, lumens: f32) -> Self {
        self.candela(lumens / (4. * std::f32::consts::PI))
    }
    /// How bright, in lumens.
    pub fn in_lumens(&self) -> f32 {
        self.candela * 4. * std::f32::consts::PI
    }
    /// How far it reaches.
    pub fn range(self, range: f32) -> Self {
        Self {
            range: range.max(1e-3),
            ..self
        }
    }
    /// How big it is: a sphere of `radius` (`PointLight::radius`).
    pub fn radius(self, radius: f32) -> Self {
        Self {
            radius: radius.max(0.),
            ..self
        }
    }
    /// A spot's cone: fully within `inner` radians of its direction,
    /// fading to nothing at `outer`. Ignored for a point light.
    pub fn cone(self, inner: f32, outer: f32) -> Self {
        Self {
            spot: self
                .spot
                .map(|spot| Spot::new(spot.direction, inner, outer)),
            ..self
        }
    }
    /// It as a `Light::Point` (its cone and radius apart).
    fn light(&self) -> Light {
        Light::Point {
            position: self.position,
            color: self.color,
            intensity: self.candela,
            range: self.range,
        }
    }
}
impl From<PointLight> for Light {
    /// Its place, colour, candela and range; a cone is set apart
    /// (`World::set_spot`) and a radius only by `World::add_point_light`.
    fn from(light: PointLight) -> Self {
        light.light()
    }
}

impl World {
    /// Shade point and spot lights through clusters (on) or, with eight or
    /// fewer, the uniform path's eight (off, the default).
    ///
    /// A world with more point lights than [`POINT_LIGHTS`](crate::POINT_LIGHTS),
    /// or any with a radius, shades through clusters either way: its view is
    /// cut into 16 by 9 tiles and 24 slices of depth, each light listed in
    /// the clusters it reaches, and each pixel shades with its own cluster's
    /// lights alone, so hundreds of lights cost about what the few reaching
    /// each pixel do. The lists are made on the CPU, again only when a
    /// light, the camera or the view's size changes. A device without
    /// storage buffers in its shaders (some GL) shades the eight nearest the
    /// camera's target, as without the `lighting` feature.
    pub fn set_clustered_lights(&self, on: bool) {
        self.change_view(|inner: &mut Inner| {
            std::mem::replace(&mut inner.lighting.clustered, on) != on
        });
    }
    /// How many lights the last frame shading through clusters left out
    /// where they reach: past the 65,536 nearest the camera, and in each
    /// cluster past its 255 nearest (a light counted once a cluster). 0
    /// unless a world holds far more lights than a view can light.
    pub fn lights_left_out(&self) -> u32 {
        self.inner.borrow().lighting.left_out.get()
    }
    /// Whether point and spot lights shade through clusters whatever their
    /// count (`set_clustered_lights`).
    pub fn clustered_lights(&self) -> bool {
        self.inner.borrow().lighting.clustered
    }
    /// Cut the sun's shadow into cascades (`None`: one map over every
    /// caster, as before). They apply while the sun casts shadows
    /// (`set_shadows`).
    ///
    /// Each cascade holds its slice of the view's bounding sphere and moves
    /// by whole texels of its map, so shadow edges hold still as the camera
    /// moves; its map is drawn again only when it moved, the sun did, or a
    /// caster in it did. The farther ones follow a moving camera at a lower
    /// rate: one of them a frame, while the nearest follows every frame.
    /// They cost `count × size² × 2` bytes (32 MB for four of 2048).
    pub fn set_cascades(&self, cascades: Option<Cascades>) {
        self.change_view(|inner: &mut Inner| {
            let cascades = cascades.map(Cascades::sanitized);
            std::mem::replace(&mut inner.lighting.cascades, cascades) != cascades
        });
    }
    /// The sun's cascades, if its shadow is cut into them.
    pub fn cascades(&self) -> Option<Cascades> {
        self.inner.borrow().lighting.cascades
    }
    /// Add `light`, a point or spot light in physical units.
    pub fn add_point_light(&self, light: PointLight) -> LightId {
        let id = self.add_light(light.light());
        self.set_point_light(id, light);
        id
    }
    /// Light `id` as `light` (a point or spot light in physical units).
    pub fn set_point_light(&self, id: LightId, light: PointLight) {
        self.set_light(id, light.light());
        self.set_spot(id, light.spot);
        self.change_view(|inner: &mut Inner| {
            if !Self::light_live(inner, id) {
                return false;
            }
            let before = inner.lighting.radius(id.index as usize, id.generation);
            // A radius not finite or under 0 is a point's.
            let radius = if light.radius.is_finite() {
                light.radius.max(0.)
            } else {
                0.
            };
            let light = PointLight { radius, ..light };
            if light.radius > 0. {
                let entry = (id.generation, light.radius);
                inner.lighting.radii.insert(id.index, entry);
            } else {
                inner.lighting.radii.remove(&id.index);
            }
            before != light.radius
        });
    }
    /// Light `id` as a point or spot light in physical units, if it's a
    /// point light still in the world.
    pub fn point_light(&self, id: LightId) -> Option<PointLight> {
        let inner = self.inner.borrow();
        let slot = inner.lights.get(id.index as usize)?;
        let Some(Light::Point {
            position,
            color,
            intensity,
            range,
        }) = slot.value.filter(|_| slot.generation == id.generation)
        else {
            return None;
        };
        let spot = inner
            .spots
            .get(&id.index)
            .filter(|(set_for, _)| *set_for == id.generation)
            .map(|(_, spot)| *spot);
        Some(PointLight {
            position,
            color,
            candela: intensity,
            range,
            radius: inner.lighting.radius(id.index as usize, id.generation),
            spot,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vec3;

    #[test]
    fn lumens_are_candela_over_the_whole_sphere() {
        let bulb = Light::point([0., 2., 0.]).lumens(800.);
        assert!((bulb.candela - 63.662).abs() < 1e-3);
        assert!((bulb.in_lumens() - 800.).abs() < 1e-2);
        // A spot's as a point's, whatever its cone.
        let spot = Light::spot([0., 2., 0.], [0., -1., 0.]).lumens(800.);
        assert_eq!(spot.candela, bulb.candela);
        assert_eq!(spot.cone(0.1, 0.2).candela, bulb.candela);
    }

    #[test]
    fn a_point_light_in_physical_units_is_todays_point_light() {
        let light = Light::point([1., 2., 3.])
            .color([1., 0.5, 0.25])
            .candela(7.)
            .range(4.);
        assert_eq!(
            Light::from(light),
            Light::Point {
                position: vec3(1., 2., 3.),
                color: [1., 0.5, 0.25],
                intensity: 7.,
                range: 4.,
            }
        );
        let world = World::new();
        let spot = Light::spot([0., 3., 0.], [0., -1., 0.])
            .lumens(4000.)
            .cone(0.3, 0.5)
            .radius(0.1);
        let id = world.add_point_light(spot);
        assert_eq!(world.point_light(id), Some(spot));
        let revision = world.revision();
        world.set_point_light(id, spot);
        assert_eq!(world.revision(), revision, "nothing changed");
        world.set_point_light(id, spot.radius(0.));
        assert_eq!(world.point_light(id).unwrap().radius, 0.);
        // A light gone and its slot taken again has no radius of the old.
        world.remove_light(id);
        assert_eq!(world.point_light(id), None);
        let other = world.add_light(Light::point([0., 1., 0.]).into());
        assert_eq!(world.point_light(other).unwrap().radius, 0.);
    }
}
