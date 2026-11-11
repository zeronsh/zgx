//! The ship-to-shore gantry crane, modelled for the demo in code and
//! written as a `.glb` (`glb.rs`), so it comes into the world as a model
//! and an animator plays its work: the trolley runs out over the ship, the
//! spreader lowers into a bay and hoists, the trolley runs back over the
//! quay and lowers again. Its ropes stretch with the spreader.
//!
//! Animators play glTF clips alone (GAPS.md, G6), so this is the design's
//! way for a demo crane (section 10: "glTF models"), not a way around it.
use crate::{
    glb::{Glb, Path, Triangles},
    shapes::{CONTAINER_40, CONTAINER_WIDTH, beam, block},
};
use zgx_3d::{AnimatorId, Geometry, Material, Model, ObjectId, Play, Transform, Vec3, World, vec3};

const PAINT: u32 = 0xb8442c;
const WHITE: u32 = 0xe4e2dc;

/// The boom's girders' top, where the trolley runs (m above the quay).
pub const TOP: f32 = 46.;
/// The trolley's height in its crane's frame.
const TROLLEY_Y: f32 = TOP + 3.6;
/// The clip's name in the model.
pub const CLIP: &str = "work";

/// Its work, keyed: seconds, the trolley along the boom (Z, m: negative
/// out over the water), the spreader's depth under the trolley (m).
const WORK: [(f32, f32, f32); 9] = [
    (0., -30., 8.),
    (6., -30., 30.),
    (10., -30., 30.),
    (16., -30., 8.),
    (26., 22., 8.),
    (32., 22., 44.),
    (36., 22., 44.),
    (42., 22., 8.),
    (48., -30., 8.),
];

/// The ropes' corners under the trolley (X, Z).
const ROPES: [(f32, f32); 4] = [(-3.2, -1.2), (3.2, -1.2), (-3.2, 1.2), (3.2, 1.2)];

/// A rope from under the trolley down to a spreader `depth` below it.
fn rope(corner: (f32, f32), depth: f32) -> (Vec3, Vec3) {
    let length = (depth - 1.65).max(0.1);
    (
        vec3(corner.0, -1.2 - length * 0.5, corner.1),
        vec3(0.08, length, 0.08),
    )
}

/// The written model, and which of its nodes are what.
struct Writer {
    glb: Glb,
    cube: Triangles,
    /// Each material's mesh of the cube, made the first time it's used.
    meshes: Vec<(Material, bool, usize)>,
    parts: usize,
}

impl Writer {
    fn part(&mut self, transform: Transform, material: Material, parent: usize) -> usize {
        self.glass_or_part(transform, material, parent, false)
    }
    fn glass_or_part(
        &mut self,
        transform: Transform,
        material: Material,
        parent: usize,
        glass: bool,
    ) -> usize {
        let mesh = match self
            .meshes
            .iter()
            .find(|(m, g, _)| *m == material && *g == glass)
        {
            Some(&(_, _, mesh)) => mesh,
            None => {
                let index = self.glb.material(&material, glass.then_some((0.85, 1.5)));
                let mesh = self.glb.mesh(self.cube, index);
                self.meshes.push((material, glass, mesh));
                mesh
            }
        };
        self.parts += 1;
        self.glb.node("part", Some(mesh), transform, Some(parent))
    }
}

/// The gantry crane as a `.glb`: legs on the rails at Z = 3 and 33, a boom
/// out over the water, an A-frame with its stays, a machinery house, and a
/// trolley with its cab, ropes and spreader; and its work as a clip.
pub fn model() -> Written {
    let unit = Geometry::cuboid(1., 1., 1.);
    let mut glb = Glb::new();
    let cube = glb.triangles(unit.vertices(), unit.indices());
    let mut w = Writer {
        glb,
        cube,
        meshes: Vec::new(),
        parts: 0,
    };
    let paint = Material::rgb(PAINT).with(0.2, 0.5);
    let white = Material::rgb(WHITE).with(0.1, 0.45);
    let dark = Material::rgb(0x2a2c30).with(0.6, 0.5);
    let root = w.glb.node("gantry", None, Transform::default(), None);
    let (half_span, water_z, land_z) = (9., 3., 33.);
    for &lx in &[-half_span, half_span] {
        for &lz in &[water_z, land_z] {
            w.part(
                beam(vec3(lx, 0.6, lz), vec3(lx, TOP, lz), 1.8, 1.8),
                paint,
                root,
            );
            w.part(block(vec3(lx, 0., lz), vec3(5., 1.2, 1.6)), dark, root);
        }
        w.part(
            beam(vec3(lx, 15., water_z), vec3(lx, 15., land_z), 1.4, 2.2),
            paint,
            root,
        );
        w.part(
            beam(vec3(lx, 15., water_z), vec3(lx, TOP - 2., land_z), 0.8, 0.8),
            paint,
            root,
        );
    }
    for &lz in &[water_z, land_z] {
        let (a, b) = (
            vec3(-half_span - 1., TOP, lz),
            vec3(half_span + 1., TOP, lz),
        );
        w.part(beam(a, b, 2.2, 2.6), paint, root);
    }
    let (out_z, back_z) = (-62., 56.);
    for &gx in &[-4.2, 4.2] {
        w.part(
            beam(
                vec3(gx, TOP + 2., back_z),
                vec3(gx, TOP + 2., out_z),
                1.3,
                3.2,
            ),
            paint,
            root,
        );
    }
    let mut z = out_z + 4.;
    while z < back_z {
        w.part(
            beam(vec3(-4.2, TOP + 0.6, z), vec3(4.2, TOP + 0.6, z), 0.5, 0.5),
            paint,
            root,
        );
        z += 9.;
    }
    let apex = |gx: f32| vec3(gx, TOP + 32., land_z - 4.);
    for &gx in &[-4.2, 4.2] {
        w.part(
            beam(vec3(gx * 1.6, TOP, land_z), apex(gx), 1.5, 1.5),
            paint,
            root,
        );
        w.part(
            beam(vec3(gx * 1.6, TOP, water_z + 8.), apex(gx), 1.2, 1.2),
            paint,
            root,
        );
        w.part(
            beam(apex(gx), vec3(gx, TOP + 3.6, -22.), 0.45, 0.45),
            white,
            root,
        );
        w.part(
            beam(apex(gx), vec3(gx, TOP + 3.6, -52.), 0.45, 0.45),
            white,
            root,
        );
        w.part(
            beam(apex(gx), vec3(gx, TOP + 3.6, back_z - 2.), 0.6, 0.6),
            white,
            root,
        );
    }
    w.part(beam(apex(-4.2), apex(4.2), 1.2, 1.2), paint, root);
    w.part(
        block(vec3(0., TOP + 3.6, 46.), vec3(12., 6.5, 15.)),
        white,
        root,
    );

    let (_, z0, d0) = WORK[0];
    let trolley = w.glb.node(
        "trolley",
        None,
        Transform::at([0., TROLLEY_Y, z0]),
        Some(root),
    );
    w.part(block(vec3(0., 0., 0.), vec3(10.5, 2.4, 7.)), white, trolley);
    let glass = Material::rgb(0xd8e4ec).with(0., 0.05);
    w.glass_or_part(
        block(vec3(5.5, -4.2, -1.5), vec3(3., 3., 3.6)),
        glass,
        trolley,
        true,
    );
    w.part(
        block(vec3(5.5, -1.3, -1.5), vec3(3.2, 1.3, 3.8)),
        white,
        trolley,
    );
    let spreader = w.glb.node(
        "spreader",
        None,
        Transform::at([0., -d0, 0.]),
        Some(trolley),
    );
    let yellow = Material::rgb(0xd9a12b).with(0.3, 0.5);
    let frame = vec3(CONTAINER_40 + 0.4, 0.9, CONTAINER_WIDTH + 0.3);
    w.part(block(vec3(0., -0.45, 0.), frame), yellow, spreader);
    let steel = Material::rgb(0x202020).with(0.8, 0.4);
    let ropes: Vec<usize> = ROPES
        .iter()
        .map(|&corner| {
            let (at, size) = rope(corner, d0);
            let t = Transform {
                position: at,
                scale: size,
                ..Transform::default()
            };
            w.part(t, steel, trolley)
        })
        .collect();

    // The work: the trolley along the boom, the spreader up and down, each
    // rope as long as the drop.
    let keys = |f: &dyn Fn(f32, f32) -> [f32; 3]| -> Vec<(f32, [f32; 3])> {
        WORK.iter().map(|&(t, z, d)| (t, f(z, d))).collect()
    };
    let mut channels = vec![
        (trolley, Path::Translation, keys(&|z, _| [0., TROLLEY_Y, z])),
        (spreader, Path::Translation, keys(&|_, d| [0., -d, 0.])),
    ];
    for (&node, &corner) in ropes.iter().zip(&ROPES) {
        channels.push((
            node,
            Path::Translation,
            keys(&|_, d| rope(corner, d).0.to_array()),
        ));
        channels.push((
            node,
            Path::Scale,
            keys(&|_, d| rope(corner, d).1.to_array()),
        ));
    }
    w.glb.animation(CLIP, &channels);
    Written {
        parts: w.parts,
        trolley,
        spreader,
        bytes: w.glb.finish(),
    }
}

/// The crane as written.
pub struct Written {
    /// The `.glb`.
    pub bytes: Vec<u8>,
    /// Its parts, each a mesh.
    pub parts: usize,
    /// Its trolley's node.
    pub trolley: usize,
    /// Its spreader's node.
    pub spreader: usize,
}

/// A crane placed in a world, and its animator.
#[derive(Clone, Copy, Debug)]
pub struct Gantry {
    /// The whole crane (moves along the quay).
    pub root: ObjectId,
    /// Its animator, playing its work (`Cranes::Still`: none).
    pub animator: Option<AnimatorId>,
}

/// How the cranes are animated: for the bench to tell the costs apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cranes {
    /// Each has an animator playing its work.
    Working,
    /// Each has an animator, playing nothing.
    Idle,
    /// None has an animator.
    Still,
}

/// `model` placed at `x` along the quay, working from `phase` seconds into
/// its cycle as `cranes` says.
pub fn place(world: &World, model: &Model, x: f32, phase: f32, cranes: Cranes) -> Gantry {
    let placed = world.place_model(model);
    world.set_transform(placed.root, Transform::at([x, 0., 0.]));
    let animator = (cranes != Cranes::Still).then(|| world.add_animator(&placed));
    if let Some(animator) = animator
        && cranes == Cranes::Working
        && let Some(clip) = model.clip(CLIP)
    {
        // A clip that can't be played leaves the crane still: nothing else.
        let _ = world.play(animator, clip, Play::new().from(phase));
    }
    Gantry {
        root: placed.root,
        animator,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crane_reads_back_and_works() {
        let written = model();
        let model = Model::from_bytes(&written.bytes).expect("the crane reads");
        let clip = model.clip(CLIP).expect("its work");
        assert!((clip.duration() - 48.).abs() < 1e-4);
        assert!(written.parts > 40, "{}", written.parts);
        let world = World::new();
        let placed = world.place_model(&model);
        let (trolley, spreader) = (
            placed.node(written.trolley).expect("the trolley"),
            placed.node(written.spreader).expect("the spreader"),
        );
        let at = |node| {
            let m = world.world_matrix(node).expect("placed").columns[3];
            vec3(m[0], m[1], m[2])
        };
        // At each key: the trolley where the work says, the spreader its
        // depth under it.
        for &(t, z, d) in &WORK[..WORK.len() - 1] {
            placed.pose(&world, clip, t);
            let (p, q) = (at(trolley), at(spreader));
            assert!(
                (p.z - z).abs() < 1e-3 && (p.y - TROLLEY_Y).abs() < 1e-3,
                "{t}: {p:?}"
            );
            assert!((p.y - q.y - d).abs() < 1e-3, "{t}: {q:?}");
        }
    }

    #[test]
    fn its_ropes_reach_the_spreader() {
        for &(_, _, depth) in &WORK {
            let (at, size) = rope(ROPES[0], depth);
            let bottom = at.y - size.y * 0.5;
            // The spreader's top is 0.45 under its frame's origin... above
            // it: its frame sits at `-depth`, its block's top at that.
            assert!((bottom - -(depth - 0.45)).abs() < 1e-4, "{bottom} {depth}");
        }
    }
}
