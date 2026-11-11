//! A small glTF 2.0 binary (`.glb`) writer, so models the demo makes in
//! code can go through zgx-3d's model path (`Model::from_bytes`) and its
//! animators, which play glTF clips alone (GAPS.md, G6). It writes what
//! the demo needs: triangle meshes (positions, normals, texture
//! coordinates, 32-bit indices), metallic-roughness materials with
//! emission and transmission, a node tree with translation, rotation and
//! scale, and linear animation channels.
use zgx_3d::{Material, Transform, Vertex};

/// A document being written.
#[derive(Default)]
pub struct Glb {
    bin: Vec<u8>,
    views: Vec<String>,
    accessors: Vec<String>,
    materials: Vec<String>,
    meshes: Vec<String>,
    nodes: Vec<Node>,
    animations: Vec<String>,
    extensions: Vec<&'static str>,
}

struct Node {
    name: String,
    mesh: Option<usize>,
    transform: Transform,
    children: Vec<usize>,
    parent: Option<usize>,
}

/// Triangles' accessors in a document: their positions, normals, texture
/// coordinates and indices.
#[derive(Clone, Copy, Debug)]
pub struct Triangles {
    position: usize,
    normal: usize,
    uv: usize,
    indices: usize,
}

/// An animation channel: the node, what it moves, and its keys (seconds,
/// value).
pub type Channel = (usize, Path, Vec<(f32, [f32; 3])>);

/// What an animation channel moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    /// Its translation (three floats a key).
    Translation,
    /// Its scale (three floats a key).
    Scale,
}

fn floats(values: impl IntoIterator<Item = f32>) -> String {
    let parts: Vec<String> = values.into_iter().map(|v| format!("{v}")).collect();
    format!("[{}]", parts.join(","))
}

fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Glb {
    /// An empty document.
    pub fn new() -> Self {
        Self::default()
    }

    /// `bytes` as a buffer view (aligned to 4), its index.
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let offset = self.bin.len();
        self.bin.extend_from_slice(bytes);
        let target = target.map_or(String::new(), |t| format!(",\"target\":{t}"));
        self.views.push(format!(
            "{{\"buffer\":0,\"byteOffset\":{offset},\"byteLength\":{}{target}}}",
            bytes.len()
        ));
        self.views.len() - 1
    }

    /// An accessor of `count` `kind`s of floats (or unsigned ints) in
    /// `view`, with its bounds where given.
    fn accessor(
        &mut self,
        view: usize,
        component: u32,
        count: usize,
        kind: &str,
        bounds: Option<(Vec<f32>, Vec<f32>)>,
    ) -> usize {
        let bounds = bounds.map_or(String::new(), |(low, high)| {
            format!(",\"min\":{},\"max\":{}", floats(low), floats(high))
        });
        self.accessors.push(format!(
            "{{\"bufferView\":{view},\"componentType\":{component},\"count\":{count},\"type\":\"{kind}\"{bounds}}}"
        ));
        self.accessors.len() - 1
    }

    /// Triangles of `vertices` and `indices`, to be meshes' geometry.
    pub fn triangles(&mut self, vertices: &[Vertex], indices: &[u32]) -> Triangles {
        let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
        let (mut p, mut n, mut t) = (Vec::new(), Vec::new(), Vec::new());
        for v in vertices {
            for i in 0..3 {
                low[i] = low[i].min(v.position[i]);
                high[i] = high[i].max(v.position[i]);
            }
            v.position.iter().for_each(|x| p.extend(x.to_le_bytes()));
            v.normal.iter().for_each(|x| n.extend(x.to_le_bytes()));
            v.uv.iter().for_each(|x| t.extend(x.to_le_bytes()));
        }
        let i: Vec<u8> = indices.iter().flat_map(|x| x.to_le_bytes()).collect();
        let count = vertices.len();
        let views = [
            self.view(&p, Some(34962)),
            self.view(&n, Some(34962)),
            self.view(&t, Some(34962)),
            self.view(&i, Some(34963)),
        ];
        Triangles {
            position: self.accessor(
                views[0],
                5126,
                count,
                "VEC3",
                Some((low.to_vec(), high.to_vec())),
            ),
            normal: self.accessor(views[1], 5126, count, "VEC3", None),
            uv: self.accessor(views[2], 5126, count, "VEC2", None),
            indices: self.accessor(views[3], 5125, indices.len(), "SCALAR", None),
        }
    }

    /// `material` (its colour and emission linear, as zgx-3d's), with
    /// light passing through it if `transmission` is given, its index.
    pub fn material(&mut self, material: &Material, transmission: Option<(f32, f32)>) -> usize {
        let mut extensions = Vec::new();
        // glTF's emissive factor stops at 1: the rest is its strength.
        let strongest = material.emissive.iter().copied().fold(0., f32::max);
        let (factor, strength) = if strongest > 1. {
            (material.emissive.map(|e| e / strongest), strongest)
        } else {
            (material.emissive, 1.)
        };
        if strength > 1. {
            extensions.push(format!(
                "\"KHR_materials_emissive_strength\":{{\"emissiveStrength\":{strength}}}"
            ));
            self.use_extension("KHR_materials_emissive_strength");
        }
        if let Some((amount, ior)) = transmission {
            extensions.push(format!(
                "\"KHR_materials_transmission\":{{\"transmissionFactor\":{amount}}}"
            ));
            extensions.push(format!("\"KHR_materials_ior\":{{\"ior\":{ior}}}"));
            self.use_extension("KHR_materials_transmission");
            self.use_extension("KHR_materials_ior");
        }
        let extensions = if extensions.is_empty() {
            String::new()
        } else {
            format!(",\"extensions\":{{{}}}", extensions.join(","))
        };
        self.materials.push(format!(
            "{{\"pbrMetallicRoughness\":{{\"baseColorFactor\":{},\"metallicFactor\":{},\"roughnessFactor\":{}}},\"emissiveFactor\":{}{extensions}}}",
            floats(material.color),
            material.metallic,
            material.roughness,
            floats(factor),
        ));
        self.materials.len() - 1
    }

    fn use_extension(&mut self, name: &'static str) {
        if !self.extensions.contains(&name) {
            self.extensions.push(name);
        }
    }

    /// A mesh of `triangles` in `material`, its index.
    pub fn mesh(&mut self, triangles: Triangles, material: usize) -> usize {
        let Triangles {
            position,
            normal,
            uv,
            indices,
        } = triangles;
        self.meshes.push(format!(
            "{{\"primitives\":[{{\"attributes\":{{\"POSITION\":{position},\"NORMAL\":{normal},\"TEXCOORD_0\":{uv}}},\"indices\":{indices},\"material\":{material}}}]}}"
        ));
        self.meshes.len() - 1
    }

    /// A node `name`, showing `mesh` if given, placed by `transform` in
    /// `parent`'s frame (the scene's if `None`), its index.
    pub fn node(
        &mut self,
        name: &str,
        mesh: Option<usize>,
        transform: Transform,
        parent: Option<usize>,
    ) -> usize {
        let index = self.nodes.len();
        self.nodes.push(Node {
            name: name.to_string(),
            mesh,
            transform,
            children: Vec::new(),
            parent,
        });
        if let Some(p) = parent {
            self.nodes[p].children.push(index);
        }
        index
    }

    /// An animation `name` of linear channels: each node, what it moves,
    /// and its keys (seconds, value).
    pub fn animation(&mut self, name: &str, channels: &[Channel]) {
        let (mut samplers, mut targets) = (Vec::new(), Vec::new());
        for (k, (node, path, keys)) in channels.iter().enumerate() {
            let times: Vec<u8> = keys.iter().flat_map(|(t, _)| t.to_le_bytes()).collect();
            let values: Vec<u8> = keys
                .iter()
                .flat_map(|(_, v)| v.iter().flat_map(|x| x.to_le_bytes()))
                .collect();
            let (first, last) = (keys[0].0, keys[keys.len() - 1].0);
            let tv = self.view(&times, None);
            let input = self.accessor(
                tv,
                5126,
                keys.len(),
                "SCALAR",
                Some((vec![first], vec![last])),
            );
            let vv = self.view(&values, None);
            let output = self.accessor(vv, 5126, keys.len(), "VEC3", None);
            samplers.push(format!(
                "{{\"input\":{input},\"output\":{output},\"interpolation\":\"LINEAR\"}}"
            ));
            let path = match path {
                Path::Translation => "translation",
                Path::Scale => "scale",
            };
            targets.push(format!(
                "{{\"sampler\":{k},\"target\":{{\"node\":{node},\"path\":\"{path}\"}}}}"
            ));
        }
        self.animations.push(format!(
            "{{\"name\":{},\"samplers\":[{}],\"channels\":[{}]}}",
            quote(name),
            samplers.join(","),
            targets.join(",")
        ));
    }

    /// The `.glb` file's bytes.
    pub fn finish(mut self) -> Vec<u8> {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let nodes: Vec<String> = self
            .nodes
            .iter()
            .map(|n| {
                let t = n.transform;
                let r = t.rotation;
                let mut s = format!(
                    "{{\"name\":{},\"translation\":{},\"rotation\":{},\"scale\":{}",
                    quote(&n.name),
                    floats(t.position.to_array()),
                    floats([r.x, r.y, r.z, r.w]),
                    floats(t.scale.to_array()),
                );
                if let Some(m) = n.mesh {
                    s.push_str(&format!(",\"mesh\":{m}"));
                }
                if !n.children.is_empty() {
                    let c: Vec<String> = n.children.iter().map(usize::to_string).collect();
                    s.push_str(&format!(",\"children\":[{}]", c.join(",")));
                }
                s.push('}');
                s
            })
            .collect();
        let roots: Vec<String> = (0..self.nodes.len())
            .filter(|&i| self.nodes[i].parent.is_none())
            .map(|i| i.to_string())
            .collect();
        let used = if self.extensions.is_empty() {
            String::new()
        } else {
            let names: Vec<String> = self.extensions.iter().map(|e| quote(e)).collect();
            format!(",\"extensionsUsed\":[{}]", names.join(","))
        };
        let animations = if self.animations.is_empty() {
            String::new()
        } else {
            format!(",\"animations\":[{}]", self.animations.join(","))
        };
        let mut json = format!(
            "{{\"asset\":{{\"version\":\"2.0\",\"generator\":\"zgx harbor demo\"}}{used},\"scene\":0,\"scenes\":[{{\"nodes\":[{}]}}],\"nodes\":[{}],\"meshes\":[{}],\"materials\":[{}],\"accessors\":[{}],\"bufferViews\":[{}],\"buffers\":[{{\"byteLength\":{}}}]{animations}}}",
            roots.join(","),
            nodes.join(","),
            self.meshes.join(","),
            self.materials.join(","),
            self.accessors.join(","),
            self.views.join(","),
            self.bin.len(),
        )
        .into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let total = 12 + 8 + json.len() + 8 + self.bin.len();
        let mut out = Vec::with_capacity(total);
        out.extend(0x4654_6C67u32.to_le_bytes()); // "glTF"
        out.extend(2u32.to_le_bytes());
        out.extend((total as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(0x4E4F_534Au32.to_le_bytes()); // "JSON"
        out.extend(json);
        out.extend((self.bin.len() as u32).to_le_bytes());
        out.extend(0x004E_4942u32.to_le_bytes()); // "BIN\0"
        out.extend(self.bin);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zgx_3d::{Geometry, Model, vec3};

    #[test]
    fn a_written_model_reads_back_with_its_nodes_and_clip() {
        let cube = Geometry::cuboid(1., 1., 1.);
        let mut g = Glb::new();
        let tri = g.triangles(cube.vertices(), cube.indices());
        let mut lamp = Material::rgb(0x202020);
        lamp.emissive = [4000., 2000., 500.];
        let paint = g.material(&Material::rgb(0xb8442c).with(0.2, 0.5), None);
        let glow = g.material(&lamp, None);
        let glass = g.material(&Material::rgb(0xffffff), Some((0.9, 1.5)));
        let (a, b, c) = (g.mesh(tri, paint), g.mesh(tri, glow), g.mesh(tri, glass));
        let root = g.node("root", Some(a), Transform::at([0., 1., 0.]), None);
        let moving = g.node("moving", Some(b), Transform::default(), Some(root));
        g.node("pane", Some(c), Transform::at([2., 0., 0.]), Some(moving));
        g.animation(
            "work",
            &[(
                moving,
                Path::Translation,
                vec![(0., [0.; 3]), (2., [0., 0., 5.])],
            )],
        );
        let model = Model::from_bytes(&g.finish()).expect("it reads");
        let clip = model.clip("work").expect("its clip");
        assert!((clip.duration() - 2.).abs() < 1e-5);
        let world = zgx_3d::World::new();
        let placed = world.place_model(&model);
        let moved = placed.node(1).expect("the moving node");
        placed.pose(&world, clip, 1.);
        let z = world.world_matrix(moved).unwrap().columns[3][2];
        assert!((z - 2.5).abs() < 1e-4, "{z}");
        let _ = vec3(0., 0., 0.);
    }
}
