//! A mesh's instance: what its draw reads of it beside its geometry.
use crate::math::Vec3;
/// Per mesh: its world matrix, its normals' (the inverse transpose, its
/// fourth row metallic, roughness, unlit), its colour, its emission.
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct Instance {
    pub(super) model: [[f32; 4]; 4],
    pub(super) normal: [[f32; 4]; 3],
    pub(super) color: [f32; 4],
    pub(super) emissive: [f32; 4],
}

/// `d`'s instance: its place, normals' matrix, material, and where its joint
/// matrices start (`base`) if it is skinned.
pub(super) fn instance(d: &crate::world::Drawn, base: u32) -> Instance {
    let m = &d.matrix.columns;
    // The inverse transpose of the upper 3x3 (its cofactors, over the
    // determinant): normals stay square to surfaces under any scale.
    let (a, b, c) = (
        Vec3::from([m[0][0], m[0][1], m[0][2]]),
        Vec3::from([m[1][0], m[1][1], m[1][2]]),
        Vec3::from([m[2][0], m[2][1], m[2][2]]),
    );
    let (x, y, z) = (b.cross(c), c.cross(a), a.cross(b));
    let determinant = a.dot(x);
    let inv = if determinant.abs() > 1e-12 {
        1. / determinant
    } else {
        0.
    };
    let material = d.material;
    let column = |v: Vec3, w: f32| [v.x * inv, v.y * inv, v.z * inv, w];
    Instance {
        model: d.matrix.columns,
        normal: [
            column(x, material.metallic.clamp(0., 1.)),
            column(y, material.roughness.clamp(0.02, 1.)),
            column(z, surface_word(d)),
        ],
        color: material
            .color
            .map(|c| if c.is_finite() { c.max(0.) } else { 0. }),
        emissive: [
            material.emissive[0],
            material.emissive[1],
            material.emissive[2],
            base as f32,
        ],
    }
}

/// `d`'s morph weights into the frame's `palette`: a matrix saying how many
/// are active and how many targets it has, then matrices of `[target,
/// weight]`s, one a column (the shader's `morph`). Joints follow.
pub(super) fn morph_block(d: &crate::world::Drawn, palette: &mut Vec<[[f32; 4]; 4]>) {
    let active = d.morph.unwrap_or_default();
    let targets = d.geometry.morphs().map_or(0, |m| m.targets());
    palette.push([
        [active.len() as f32, targets as f32, 0., 0.],
        [0.; 4],
        [0.; 4],
        [0.; 4],
    ]);
    for pairs in active.chunks(4) {
        palette.push(std::array::from_fn(|column| {
            let [target, weight] = pairs.get(column).copied().unwrap_or([0.; 2]);
            [target, weight, 0., 0.]
        }));
    }
}

/// The shader's third material word: unlit from 1 up, lit below, with the
/// alpha cutoff as how far from there (`cutoff_of` undoes it): 1 + c, or −c.
pub(super) fn surface_word(d: &crate::world::Drawn) -> f32 {
    let cutoff = d.surface.alpha_cutoff.map_or(0., |c| c.max(0.));
    if d.material.unlit {
        1. + cutoff
    } else {
        -cutoff
    }
}
