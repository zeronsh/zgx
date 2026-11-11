//! The harbor's world, built from a seed: the same seed and config build
//! the same objects in the same order, so the bench's frames repeat.
//!
//! Layout (metres): +X runs along the quay, the land is +Z, the water -Z.
//! The quay's edge is at Z = 0, its deck at Y = 0, the water at
//! [`WATER_LEVEL`]. The apron under the cranes runs to Z = 60, the yard
//! beyond it.
use crate::{
    rng::Rng,
    shapes::{self, CONTAINER_20, CONTAINER_40, CONTAINER_HEIGHT, CONTAINER_WIDTH, beam, block},
    sky, textures,
};
use zgx_3d::{
    Camera, Cascades, Clearcoat, Environment, Geometry, Light, LightId, Map, MapPlacement,
    Material, Object, ObjectId, PointLight, Quat, ToneMapping, Transform, Transmission, Vec3,
    World, vec3,
};

/// The water's surface, below the quay's deck.
pub const WATER_LEVEL: f32 = -3.;
/// The ship's centreline, off the quay.
pub const SHIP_Z: f32 = -24.;
/// The quay's length along X.
pub const QUAY_LENGTH: f32 = 640.;
/// The land's depth along +Z.
pub const LAND_DEPTH: f32 = 380.;

/// What to build.
#[derive(Clone, Debug)]
pub struct Config {
    /// Every random choice follows it.
    pub seed: u64,
    /// Container blocks in the yard, along X and along Z.
    pub blocks: (usize, usize),
    /// The highest a yard stack goes.
    pub max_tiers: usize,
    /// Ship-to-shore gantry cranes on the quay.
    pub gantries: usize,
    /// High masts in the yard: along each lane, and lanes. Each carries
    /// four sodium floods.
    pub masts: (usize, usize),
    /// Every lamp's reach, as a share of where its light falls to
    /// [`DARK_LUX`] (1: there; 0: no lamps built at all).
    pub lamp_reach: f32,
    /// The sun's elevation and azimuth, degrees (`sky::sun_toward`).
    pub sun: (f32, f32),
    /// It rains (the emitters are made; the panel turns it on and off).
    pub rain: bool,
    /// The wind, m/s.
    pub wind: [f32; 3],
    /// How the cranes are animated.
    pub cranes: Cranes,
    /// The sea and the wet are drawn by the demo's materials; off, by the
    /// stock one (flat and glossy).
    pub water: bool,
    /// Which of the water's parts are made.
    pub water_parts: crate::water::Parts,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            seed: 0x4841_5242_4f52,
            blocks: (5, 7),
            max_tiers: 5,
            gantries: 4,
            masts: (11, 4),
            lamp_reach: 1.,
            sun: (7., 28.),
            rain: true,
            wind: [2.5, 0., 1.],
            cranes: Cranes::Working,
            water: true,
            water_parts: Default::default(),
        }
    }
}

/// The exposure, in stops down, that brings the harbor's physical light to
/// the display: lamps are in lumens and the sun in lux (a point light's
/// candela over its distance squared is on the sun's scale), so a dawn
/// lit by both is brought down by this much (EV 11: 2048 lux shows as 1).
/// The sky's radiance and emissive surfaces are given on the same scale.
pub const EV: f32 = 11.;
/// One displayed unit of light at [`EV`], in lux.
pub const UNIT: f32 = 2048.;

/// The sun's illuminance with the sun high, lux (a hazy morning's; the
/// clear midday sun gives about 100,000).
const SUN_LUX: f32 = 6500.;

/// The sun as a light at `elevation` and `azimuth` degrees, in lux.
pub fn sun_light(elevation: f32, azimuth: f32) -> Light {
    let toward = sky::sun_toward(elevation, azimuth);
    // Dimmer near the horizon (more air to cross), and gone a degree
    // under it.
    let risen = ((elevation + 1.) / 3.).clamp(0., 1.);
    let strength = SUN_LUX * (0.35 + 0.65 * (elevation / 25.).clamp(0., 1.)) * risen;
    Light::Directional {
        direction: -toward,
        color: sky::sun_color(elevation),
        intensity: strength,
    }
}

/// The sun's shadow in four cascades over the view's first 600 m: the
/// whole quay and yard from anywhere on the fly-through's low passes.
pub fn cascades() -> Cascades {
    Cascades::new(4).distance(600.)
}

/// High-pressure sodium's colour, linear.
const SODIUM: [f32; 3] = [1., 0.55, 0.2];
/// A white LED flood's colour, linear.
const LED: [f32; 3] = [0.86, 0.93, 1.];

/// The illuminance a lamp's reach ends at, lux: about a full moon's.
pub const DARK_LUX: f32 = 0.5;

/// Where `light`, straight ahead of it, falls to [`DARK_LUX`].
pub fn reach(light: &PointLight) -> f32 {
    (light.candela / DARK_LUX).sqrt()
}

/// `light` on, or off (no lumens).
pub fn lamp(light: PointLight, on: bool) -> PointLight {
    if on { light } else { light.lumens(0.) }
}

/// The harbor's handles: what the settings and animations reach.
#[derive(Clone, Debug)]
pub struct Harbor {
    /// What it was built from.
    pub config: Config,
    /// The sun.
    pub sun: LightId,
    /// The sky's light from all round (when no environment is set).
    pub ambient: LightId,
    /// The lamps, each as lit.
    pub lamps: Vec<(LightId, PointLight)>,
    /// The yard's containers, in the order built (the ship's are apart).
    pub yard: Vec<ObjectId>,
    /// The containers on the ship.
    pub deck: Vec<ObjectId>,
    /// The gantry cranes, each a placed model on an animator.
    pub gantries: Vec<Gantry>,
    /// The water's surface.
    pub water: ObjectId,
    /// Where the ship's funnel is (its generator's sound).
    pub funnel: ObjectId,
    /// The buoy out in the water (its bell).
    pub buoy: ObjectId,
    /// The sea's and the wet's materials, if made (`Config::water`).
    pub sea: Option<crate::water::Water>,
    /// The rain, if it was made (`Config::rain`, and a device for it).
    pub rain: Option<crate::weather::Rain>,
    /// Objects added, of every kind.
    pub objects: usize,
    /// Triangles in every mesh added (as made, each instance counted).
    pub triangles: usize,
}

pub use crate::crane::{Cranes, Gantry};

/// The shared meshes: each geometry is made once, so its instances draw
/// in one call.
struct Kit {
    unit: Geometry,
    cylinder: Geometry,
    box20: Geometry,
    box40: Geometry,
    wear: zgx_3d::Texture,
}

struct Build<'a> {
    world: &'a World,
    kit: Kit,
    rng: Rng,
    objects: usize,
    triangles: usize,
}

impl Build<'_> {
    fn add(&mut self, object: Object) -> ObjectId {
        self.objects += 1;
        if let Some(g) = &object.geometry {
            self.triangles += g.indices().len() / 3;
        }
        self.world.add(object)
    }
    /// A part from the unit cube, placed by `transform` under `parent`.
    fn part(&mut self, transform: Transform, material: Material, parent: ObjectId) -> ObjectId {
        let unit = self.kit.unit.clone();
        self.add(
            Object::mesh(unit, material)
                .transformed(transform)
                .child_of(parent),
        )
    }
    fn group(&mut self, at: Vec3, parent: Option<ObjectId>) -> ObjectId {
        let mut g = Object::group().at(at);
        if let Some(p) = parent {
            g = g.child_of(p);
        }
        self.add(g)
    }
    /// A container `forty` feet long or twenty, its bottom's middle at
    /// `at`, turned `yaw` about Y, in a colour from the palette.
    fn container(&mut self, forty: bool, at: Vec3, yaw: f32, parent: ObjectId) -> ObjectId {
        let geometry = if forty {
            self.kit.box40.clone()
        } else {
            self.kit.box20.clone()
        };
        let hex = PALETTE[self.rng.below(PALETTE.len())];
        let mut m = Material::rgb(hex).with(0.15, 0.62);
        let shade = self.rng.range(0.82, 1.08);
        for c in &mut m.color[..3] {
            *c *= shade;
        }
        let id = self.add(
            Object::mesh(geometry, m)
                .transformed(Transform {
                    position: at,
                    rotation: Quat::from_axis_angle(Vec3::Y, yaw),
                    scale: Vec3::ONE,
                })
                .child_of(parent),
        );
        self.world.set_texture(id, Some(self.kit.wear.clone()));
        id
    }
}

/// Container paints: muted, worn industrial colours (sRGB), none of them
/// any company's.
const PALETTE: [u32; 12] = [
    0x8c2f1f, 0x1f3d6b, 0x1d6b66, 0xc4651a, 0x8a8f94, 0xd8d8d2, 0x3f6b35, 0x5e1f2a, 0xc9a227,
    0x6b4a2f, 0x7a3a5e, 0x5d8fb8,
];

impl Harbor {
    /// Build the harbor into `world` (which should be empty), lit by
    /// `environment` if given, else by a hemisphere of the sky's colours.
    pub fn build(world: &World, config: &Config, environment: Option<Environment>) -> Self {
        let kit = Kit {
            unit: Geometry::cuboid(1., 1., 1.),
            cylinder: Geometry::cylinder(0.5, 1., 20),
            box20: shapes::container(CONTAINER_20),
            box40: shapes::container(CONTAINER_40),
            wear: textures::container_wear(config.seed),
        };
        let mut b = Build {
            world,
            kit,
            rng: Rng::new(config.seed),
            objects: 0,
            triangles: 0,
        };
        world.set_background(Some(sky::BACKGROUND));
        world.set_tone_mapping(ToneMapping::Aces);
        world.set_shadows(true);
        world.set_cascades(Some(cascades()));
        world.set_exposure(-EV);
        let has_environment = environment.is_some();
        world.set_environment(environment.map(|e| e.intensity(UNIT)));
        let (elevation, azimuth) = config.sun;
        let sun = world.add_light(sun_light(elevation, azimuth));
        let ambient = world.add_light(Light::Hemisphere {
            sky: [0.36, 0.42, 0.62],
            ground: [0.12, 0.1, 0.09],
            intensity: if has_environment { 0. } else { 0.7 * UNIT },
        });
        world.set_camera(Camera {
            near: 0.5,
            far: 4000.,
            ..Camera::looking_at([-160., 60., -160.], [0., 10., 60.])
        });

        let water = quay(&mut b);
        let yard = yard(&mut b, config);
        // The cranes: one model, written in code, placed along the quay,
        // each working a quarter of its cycle after the last.
        let written = crate::crane::model();
        let parts = written.parts;
        let crane = crate::crane::Placing {
            model: zgx_3d::Model::from_bytes(&written.bytes).expect("the crane's model reads"),
            trolley: written.trolley,
        };
        let gantries: Vec<Gantry> = (0..config.gantries)
            .map(|i| {
                let x = (i as f32 - (config.gantries as f32 - 1.) * 0.5) * 66.;
                b.objects += parts + 3;
                b.triangles += parts * 12;
                crate::crane::place(world, &crane, x, i as f32 * 12., config.cranes)
            })
            .collect();
        let (ship_root, deck) = ship(&mut b);
        let funnel = b.group(vec3(-101., 30., 0.), Some(ship_root));
        let buoy = buoy(&mut b);
        let lamps = lamps(&mut b, config, &gantries, ship_root);
        // The shore across the water.
        let ridge = shapes::ridge(-1500., 5000., WATER_LEVEL - 1., 90., config.seed as u32);
        b.add(Object::mesh(ridge, Material::rgb(0x3c4250).with(0., 0.95)));
        let rain = config
            .rain
            .then(|| crate::weather::Rain::new(world, config.wind));
        let rain = match rain {
            Some(Err(error)) => {
                eprintln!("harbor: no rain: {error}");
                None
            }
            other => other.and_then(Result::ok),
        };
        let sea = match config
            .water
            .then(|| crate::water::Water::new(world, water, config.wind, config.water_parts))
        {
            Some(Ok(sea)) => {
                b.objects += 1;
                b.triangles += crate::water::Water::triangles();
                Some(sea)
            }
            Some(Err(error)) => {
                eprintln!("harbor: water as the stock material: {error}");
                None
            }
            None => None,
        };
        Self {
            sea,
            funnel,
            buoy,
            rain,
            config: config.clone(),
            sun,
            ambient,
            lamps,
            yard,
            deck,
            gantries,
            water,
            objects: b.objects,
            triangles: b.triangles,
        }
    }
}

/// The quay, apron, yard surface, wall, fenders, bollards, rails and the
/// water. Returns the water.
fn quay(b: &mut Build) -> ObjectId {
    let world = b.world;
    let root = b.group(Vec3::ZERO, None);
    // The land: an apron of concrete slabs, then the yard's asphalt, both
    // slabs 12 m deep so the quay wall shows down to the water.
    let apron_depth = 60.;
    let apron = b.add(
        Object::mesh(b.kit.unit.clone(), Material::rgb(0xffffff).with(0., 0.85))
            .transformed(block(
                vec3(0., -12., apron_depth * 0.5),
                vec3(QUAY_LENGTH, 12., apron_depth),
            ))
            .child_of(root),
    );
    world.set_texture(apron, Some(textures::concrete(b.rng.next_u64())));
    world.set_map_placement(
        apron,
        Map::BaseColour,
        Some(MapPlacement::new().scaled(QUAY_LENGTH / 10., apron_depth / 10.)),
    );
    let yard_depth = LAND_DEPTH - apron_depth;
    let ground = b.add(
        Object::mesh(b.kit.unit.clone(), Material::rgb(0xffffff).with(0., 0.9))
            .transformed(block(
                vec3(0., -12., apron_depth + yard_depth * 0.5),
                vec3(QUAY_LENGTH, 12., yard_depth),
            ))
            .child_of(root),
    );
    world.set_texture(ground, Some(textures::asphalt(b.rng.next_u64())));
    world.set_map_placement(
        ground,
        Map::BaseColour,
        Some(MapPlacement::new().scaled(QUAY_LENGTH / 8., yard_depth / 8.)),
    );
    // A cope along the edge, fenders on the wall and bollards on the cope.
    let cope = Material::rgb(0x9a968c).with(0., 0.8);
    b.part(
        block(vec3(0., 0., 0.6), vec3(QUAY_LENGTH, 0.25, 1.2)),
        cope,
        root,
    );
    let fender = Material::rgb(0x161616).with(0., 0.7);
    let bollard = Material::rgb(0x23272b).with(0.6, 0.45);
    let mut x = -QUAY_LENGTH * 0.5 + 8.;
    while x < QUAY_LENGTH * 0.5 {
        b.part(
            block(vec3(x, -2.8, -0.45), vec3(2.2, 2.6, 0.9)),
            fender,
            root,
        );
        let cylinder = b.kit.cylinder.clone();
        b.add(
            Object::mesh(cylinder, bollard)
                .transformed(Transform {
                    position: vec3(x + 8., 0.6, 1.),
                    rotation: Quat::IDENTITY,
                    scale: vec3(0.7, 0.9, 0.7),
                })
                .child_of(root),
        );
        x += 16.;
    }
    // Crane rails, and painted lines: the apron's edge and the lanes.
    let steel = Material::rgb(0x55585c).with(1., 0.35);
    for z in [3., 33.] {
        b.part(
            block(vec3(0., 0., z), vec3(QUAY_LENGTH, 0.18, 0.25)),
            steel,
            root,
        );
    }
    let paint = Material::rgb(0xd8c25a).with(0., 0.6);
    let white = Material::rgb(0xd6d6d0).with(0., 0.6);
    for z in [1.8, 38., 58.] {
        b.part(
            block(vec3(0., 0., z), vec3(QUAY_LENGTH, 0.02, 0.2)),
            paint,
            root,
        );
    }
    for k in 0..40 {
        let x = -QUAY_LENGTH * 0.5 + 8. + k as f32 * 16.;
        b.part(block(vec3(x, 0., 48.), vec3(6., 0.02, 0.15)), white, root);
    }
    // The water: a flat, glossy placeholder (the water material comes with
    // custom materials: GAPS.md).
    b.add(
        Object::mesh(
            Geometry::plane(6000., 6000.),
            Material::rgb(0x18323a).with(0., 0.06),
        )
        .at([0., WATER_LEVEL, -2900.]),
    )
}

/// The yard: blocks of stacked containers between lanes. Returns them.
fn yard(b: &mut Build, config: &Config) -> Vec<ObjectId> {
    let root = b.group(Vec3::ZERO, None);
    let (rows, bays) = (6, 8);
    let row_pitch = CONTAINER_WIDTH + 0.32;
    let bay_pitch = CONTAINER_40 + 0.7;
    let block_x = bays as f32 * bay_pitch + 20.;
    let block_z = rows as f32 * row_pitch + 14.;
    let mut out = Vec::new();
    let (nx, nz) = config.blocks;
    for bz in 0..nz {
        for bx in 0..nx {
            let origin = vec3(
                (bx as f32 - (nx as f32 - 1.) * 0.5) * block_x,
                0.,
                76. + bz as f32 * block_z,
            );
            // A block's fill: some blocks full, some half worked.
            let fill = b.rng.range(0.45, 1.);
            for bay in 0..bays {
                for row in 0..rows {
                    let at = origin
                        + vec3(
                            (bay as f32 - (bays as f32 - 1.) * 0.5) * bay_pitch,
                            0.,
                            (row as f32 - (rows as f32 - 1.) * 0.5) * row_pitch,
                        );
                    let tiers = if b.rng.chance(fill) {
                        1 + b.rng.below(config.max_tiers)
                    } else {
                        0
                    };
                    // A bay of 20 ft pairs, or of 40 ft boxes, all the way up.
                    let twenties = b.rng.chance(0.3);
                    for tier in 0..tiers {
                        let y = tier as f32 * CONTAINER_HEIGHT;
                        // Stacked a little out of true.
                        let nudge = vec3(b.rng.range(-0.06, 0.06), y, b.rng.range(-0.04, 0.04));
                        let yaw = b.rng.range(-0.004, 0.004);
                        if twenties {
                            for side in [-1., 1.] {
                                let x = side * (CONTAINER_20 * 0.5 + 0.04);
                                out.push(b.container(
                                    false,
                                    at + nudge + vec3(x, 0., 0.),
                                    yaw,
                                    root,
                                ));
                            }
                        } else {
                            out.push(b.container(true, at + nudge, yaw, root));
                        }
                    }
                }
            }
        }
    }
    out
}

/// A channel buoy out in the water: a red can on a float, a light on top.
fn buoy(b: &mut Build) -> ObjectId {
    let root = b.group(vec3(-70., WATER_LEVEL, -140.), None);
    let red = Material::rgb(0xb3261e).with(0.1, 0.5);
    let cylinder = b.kit.cylinder.clone();
    b.add(
        Object::mesh(cylinder, red)
            .transformed(Transform {
                position: vec3(0., 0.6, 0.),
                rotation: Quat::IDENTITY,
                scale: vec3(2.4, 1.6, 2.4),
            })
            .child_of(root),
    );
    b.part(block(vec3(0., 1.4, 0.), vec3(1.2, 2.6, 1.2)), red, root);
    let mut lamp = Material::rgb(0x301010).with(0., 0.4);
    lamp.emissive = [3. * UNIT, 0.4 * UNIT, 0.3 * UNIT];
    b.part(block(vec3(0., 4., 0.), vec3(0.4, 0.4, 0.4)), lamp, root);
    root
}

/// The ship at its berth: hull, hatch covers, deck stacks, the
/// accommodation block with its bridge and funnel. Returns its group and
/// the deck's containers.
fn ship(b: &mut Build) -> (ObjectId, Vec<ObjectId>) {
    let (length, beam_width, deck) = (232., 32.2, 9.);
    let root = b.group(vec3(0., 0., SHIP_Z), None);
    let hull = shapes::hull(
        length,
        beam_width,
        deck,
        -15.,
        WATER_LEVEL - 1.5,
        [0.018, 0.04, 0.075, 1.],
    );
    let hull_id = b.add(Object::mesh(hull, Material::rgb(0xffffff).with(0.2, 0.42)).child_of(root));
    b.world
        .set_clearcoat(hull_id, Some(Clearcoat::new(0.5).roughness(0.25)));
    // Hatch covers and the stacks on them, bay by bay; rows fit the beam.
    let hatch = Material::rgb(0x4d5a52).with(0.3, 0.6);
    let mut out = Vec::new();
    let bay_pitch = CONTAINER_40 + 1.2;
    for bay in 0..12 {
        let x = -70. + bay as f32 * bay_pitch;
        let rows = match bay {
            11 => 8,
            10 => 10,
            _ => 12,
        };
        let width = rows as f32 * (CONTAINER_WIDTH + 0.05);
        b.part(
            block(
                vec3(x, deck, 0.),
                vec3(CONTAINER_40 + 0.6, 1.1, width + 0.6),
            ),
            hatch,
            root,
        );
        let tiers = 2 + b.rng.below(4);
        for row in 0..rows {
            let z = (row as f32 - (rows as f32 - 1.) * 0.5) * (CONTAINER_WIDTH + 0.05);
            let here = tiers.saturating_sub(usize::from(b.rng.chance(0.25)));
            for tier in 0..here {
                let y = deck + 1.1 + tier as f32 * CONTAINER_HEIGHT;
                out.push(b.container(true, vec3(x, y, z), 0., root));
            }
        }
    }
    // Accommodation block aft, its bridge across the full beam on top.
    let white = Material::rgb(0xe9e7e0).with(0., 0.55);
    let (ax, block_top) = (-92., deck + 22.);
    b.part(block(vec3(ax, deck, 0.), vec3(13., 22., 24.)), white, root);
    b.part(
        block(
            vec3(ax + 1., block_top, 0.),
            vec3(10., 3.2, beam_width + 1.),
        ),
        white,
        root,
    );
    // The bridge's windows: glass over a dark room, along its forward face.
    let room = Material::rgb(0x0c0f12).with(0., 0.9);
    b.part(
        block(
            vec3(ax + 5.4, block_top + 0.9, 0.),
            vec3(0.6, 1.6, beam_width - 1.),
        ),
        room,
        root,
    );
    let glass = b.part(
        block(
            vec3(ax + 6.1, block_top + 0.85, 0.),
            vec3(0.12, 1.75, beam_width - 0.6),
        ),
        Material::rgb(0xcfe0e8).with(0., 0.04),
        root,
    );
    b.world
        .set_transmission(glass, Some(Transmission::new(0.9).ior(1.52)));
    // Cabin windows down the block's forward face: some lit.
    for level in 0..6 {
        for k in 0..7 {
            let lit = b.rng.chance(0.3);
            let mut m = Material::rgb(0x111518).with(0., 0.1);
            if lit {
                m.emissive = [1.6 * UNIT, 1.1 * UNIT, 0.55 * UNIT];
            }
            let z = (k as f32 - 3.) * 3.;
            let y = deck + 3. + level as f32 * 3.2;
            b.part(block(vec3(ax + 6.55, y, z), vec3(0.12, 1.1, 1.4)), m, root);
        }
    }
    // Funnel, with a band.
    b.part(
        block(vec3(ax - 9., deck + 18., 0.), vec3(7., 14., 9.)),
        Material::rgb(0x2b2f36).with(0.2, 0.5),
        root,
    );
    b.part(
        block(vec3(ax - 9., deck + 27.5, 0.), vec3(7.1, 2., 9.1)),
        Material::rgb(0xc9a227).with(0.1, 0.5),
        root,
    );
    // A foremast at the bow.
    let cylinder = b.kit.cylinder.clone();
    b.add(
        Object::mesh(cylinder, Material::rgb(0xd8d6cf).with(0.3, 0.5))
            .transformed(Transform {
                position: vec3(length * 0.5 - 14., deck + 10., 0.),
                rotation: Quat::IDENTITY,
                scale: vec3(0.8, 16., 0.8),
            })
            .child_of(root),
    );
    (root, out)
}

/// The lights of a working night not yet over: high masts over the yard's
/// lanes with four sodium floods each, posts along the quay, floods under
/// each gantry's boom (following the crane), and the ship's deck lights.
/// Some hundreds in all, shaded through clusters.
fn lamps(
    b: &mut Build,
    config: &Config,
    gantries: &[Gantry],
    ship: ObjectId,
) -> Vec<(LightId, PointLight)> {
    let root = b.group(Vec3::ZERO, None);
    let pole = Material::rgb(0x8d9196).with(0.8, 0.4);
    let mut glow = Material::rgb(0x222222).with(0.2, 0.5);
    glow.emissive = [3.5 * UNIT, 1.9 * UNIT, 0.7 * UNIT];
    let mut white = Material::rgb(0x222222).with(0.2, 0.5);
    white.emissive = [2.6 * UNIT, 2.8 * UNIT, 3. * UNIT];
    let mut out = Vec::new();
    if config.lamp_reach <= 0. {
        return out;
    }
    let reach = config.lamp_reach;
    let mut add = |b: &mut Build, light: PointLight, parent: Option<ObjectId>| {
        let light = light.range(self::reach(&light) * reach);
        let id = b.world.add_point_light(light);
        if parent.is_some() {
            b.world.set_light_parent(id, parent);
        }
        out.push((id, light));
    };
    // High masts: in the lanes between the yard's block rows, at block
    // ends and middles along them.
    let block_z = 6. * (CONTAINER_WIDTH + 0.32) + 14.;
    let block_x = 8. * (CONTAINER_40 + 0.7) + 20.;
    let (along, lanes) = config.masts;
    for lane in 0..lanes {
        let z = 76. + (lane as f32 * 2. - 0.5) * block_z;
        for k in 0..along {
            let x = (k as f32 - (along as f32 - 1.) * 0.5) * block_x * 0.5;
            let height = 35.;
            b.part(
                beam(vec3(x, 0., z), vec3(x, height, z), 0.7, 0.7),
                pole,
                root,
            );
            b.part(block(vec3(x, height, z), vec3(4., 0.5, 4.)), pole, root);
            for (dx, dz) in [(1., 1.), (1., -1.), (-1., 1.), (-1., -1.)] {
                let at = vec3(x + dx * 1.6, height - 0.3, z + dz * 1.6);
                b.part(
                    block(at - vec3(0., 0.35, 0.), vec3(0.9, 0.35, 0.9)),
                    glow,
                    root,
                );
                let light = Light::spot(at, vec3(dx, -2.2, dz))
                    .lumens(32_000.)
                    .color(SODIUM)
                    .cone(0.55, 0.95)
                    .radius(0.25);
                add(b, light, None);
            }
        }
    }
    // Posts along the apron's back edge.
    let mut x = -QUAY_LENGTH * 0.5 + 20.;
    while x < QUAY_LENGTH * 0.5 {
        let (z, height) = (56., 12.);
        b.part(
            beam(vec3(x, 0., z), vec3(x, height, z), 0.3, 0.3),
            pole,
            root,
        );
        b.part(
            block(vec3(x, height, z - 0.6), vec3(0.6, 0.3, 1.4)),
            glow,
            root,
        );
        let light = Light::point(vec3(x, height - 0.2, z - 0.8))
            .lumens(14_000.)
            .color(SODIUM)
            .radius(0.15);
        add(b, light, None);
        x += 40.;
    }
    // Floods under each boom, in the crane's frame.
    for g in gantries {
        for z in [-56., -40., -24., -6., 12.] {
            let at = vec3(0., 47.2, z);
            b.part(block(at, vec3(1.2, 0.4, 0.8)), white, g.root);
            let light = Light::spot(at, vec3(0., -1., 0.))
                .lumens(24_000.)
                .color(LED)
                .cone(0.4, 0.7)
                .radius(0.2);
            add(b, light, Some(g.root));
        }
    }
    // The ship's deck lights, in its frame.
    for k in 0..9 {
        let at = vec3(-75. + k as f32 * 18., 10.8, 0.);
        let light = Light::point(at)
            .lumens(6_000.)
            .color([1., 0.86, 0.66])
            .radius(0.1);
        add(b, light, Some(ship));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_builds_the_same_harbor() {
        let (a, b) = (World::new(), World::new());
        let config = Config {
            blocks: (2, 2),
            ..Config::default()
        };
        let ha = Harbor::build(&a, &config, None);
        let hb = Harbor::build(&b, &config, None);
        assert_eq!(ha.objects, hb.objects);
        assert_eq!(ha.triangles, hb.triangles);
        assert_eq!(ha.yard.len(), hb.yard.len());
        for (x, y) in ha.yard.iter().zip(&hb.yard) {
            assert_eq!(
                a.object(*x).unwrap().material,
                b.object(*y).unwrap().material
            );
            assert_eq!(
                a.object(*x).unwrap().transform,
                b.object(*y).unwrap().transform
            );
        }
    }

    #[test]
    fn the_default_yard_holds_thousands_of_containers() {
        let world = World::new();
        let h = Harbor::build(&world, &Config::default(), None);
        let containers = h.yard.len() + h.deck.len();
        assert!(containers > 3000, "{containers}");
        assert!(
            h.lamps.len() > 200,
            "lamps by the hundred: {}",
            h.lamps.len()
        );
    }
}
