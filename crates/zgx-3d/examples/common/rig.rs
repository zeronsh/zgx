//! A generated skinned model for animation benches and tests: `joints`
//! joints in chains of eight off a spine, a triangle bound to each, and a
//! clip ("Wave") two seconds long baked at 30 keys a second turning and
//! moving every joint, as an exporter writes a character's (`.gltf`, its
//! buffer inline). Nothing fetched: the same bytes everywhere.
#![allow(dead_code)]

/// Joint `i`'s parent.
fn parent(i: usize) -> Option<usize> {
    match i {
        0 => None,
        i if i.is_multiple_of(8) => Some(i - 8),
        i => Some(i - 1),
    }
}
/// Joint `i`'s place in its parent's frame.
fn local(i: usize) -> [f32; 3] {
    if i.is_multiple_of(8) {
        [0., 0.2, 0.]
    } else {
        [0.1, 0.05, 0.]
    }
}

pub fn rig(joints: usize) -> Vec<u8> {
    let joints = joints.max(1);
    let keys = 60;
    let mut world = vec![[0f32; 3]; joints];
    for i in 0..joints {
        let l = local(i);
        let p = parent(i).map_or([0.; 3], |p| world[p]);
        world[i] = [p[0] + l[0], p[1] + l[1], p[2] + l[2]];
    }
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut push = |bytes: Vec<u8>, count: usize, kind: &str, component: u32| {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        views.push(format!(
            r#"{{"buffer":0,"byteOffset":{},"byteLength":{}}}"#,
            bin.len(),
            bytes.len()
        ));
        bin.extend(bytes);
        accessors.push(format!(
            r#"{{"bufferView":{},"componentType":{component},"count":{count},"type":"{kind}"}}"#,
            views.len() - 1
        ));
        accessors.len() - 1
    };
    let floats = |v: &[f32]| v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>();
    let (mut positions, mut ids, mut weights) = (Vec::new(), Vec::new(), Vec::new());
    for (i, p) in world.iter().enumerate() {
        for corner in [[0., 0., 0.], [0.05, 0., 0.], [0., 0.05, 0.]] {
            positions.extend([p[0] + corner[0], p[1] + corner[1], p[2] + corner[2]]);
            ids.extend((i as u16).to_le_bytes());
            ids.extend([0u8; 6]);
            weights.extend([1f32, 0., 0., 0.]);
        }
    }
    let vertices = joints * 3;
    let position = push(floats(&positions), vertices, "VEC3", 5126);
    let joint = push(ids, vertices, "VEC4", 5123);
    let weight = push(floats(&weights), vertices, "VEC4", 5126);
    let inverse: Vec<f32> = world
        .iter()
        .flat_map(|p| {
            [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., -p[0], -p[1], -p[2], 1.,
            ]
        })
        .collect();
    let inverse = push(floats(&inverse), joints, "MAT4", 5126);
    let times: Vec<f32> = (0..keys).map(|k| k as f32 / 30.).collect();
    let input = push(floats(&times), keys, "SCALAR", 5126);
    let (mut samplers, mut channels) = (Vec::new(), Vec::new());
    for i in 0..joints {
        let phase = i as f32 * 0.37;
        let at = |k: usize| k as f32 / keys as f32 * std::f32::consts::TAU + phase;
        let turns: Vec<f32> = (0..keys)
            .flat_map(|k| {
                let (s, c) = (0.15 * at(k).sin()).sin_cos();
                [0., 0., s, c]
            })
            .collect();
        let l = local(i);
        let moves: Vec<f32> = (0..keys)
            .flat_map(|k| [l[0] + 0.01 * at(k).cos(), l[1], l[2]])
            .collect();
        for (values, kind, path) in [(turns, "VEC4", "rotation"), (moves, "VEC3", "translation")] {
            let output = push(floats(&values), keys, kind, 5126);
            samplers.push(format!(r#"{{"input":{input},"output":{output}}}"#));
            channels.push(format!(
                r#"{{"sampler":{},"target":{{"node":{},"path":"{path}"}}}}"#,
                samplers.len() - 1,
                i + 1
            ));
        }
    }
    // Node 0 holds the mesh; joint i is node i + 1.
    let mut nodes = vec![r#"{"mesh":0,"skin":0}"#.to_string()];
    for i in 0..joints {
        let children: Vec<String> = (0..joints)
            .filter(|&c| parent(c) == Some(i))
            .map(|c| (c + 1).to_string())
            .collect();
        let [x, y, z] = local(i);
        let children = if children.is_empty() {
            String::new()
        } else {
            format!(r#","children":[{}]"#, children.join(","))
        };
        nodes.push(format!(r#"{{"translation":[{x},{y},{z}]{children}}}"#));
    }
    let joint_nodes: Vec<String> = (1..=joints).map(|j| j.to_string()).collect();
    let doc = format!(
        concat!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0,1]}}],"nodes":[{}],"#,
            r#""meshes":[{{"primitives":[{{"attributes":{{"POSITION":{},"JOINTS_0":{},"WEIGHTS_0":{}}}}}]}}],"#,
            r#""skins":[{{"joints":[{}],"inverseBindMatrices":{}}}],"#,
            r#""animations":[{{"name":"Wave","samplers":[{}],"channels":[{}]}}],"#,
            r#""accessors":[{}],"bufferViews":[{}],"#,
            r#""buffers":[{{"byteLength":{},"uri":"data:application/octet-stream;base64,{}"}}]}}"#
        ),
        nodes.join(","),
        position,
        joint,
        weight,
        joint_nodes.join(","),
        inverse,
        samplers.join(","),
        channels.join(","),
        accessors.join(","),
        views.join(","),
        bin.len(),
        base64(&bin),
    );
    doc.into_bytes()
}

fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                DIGITS[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

/// A chain of `joints` nodes, each 0.1 above the last (node 0 the chain's
/// first), and a clip ("Sway") turning each about z as world_bench's
/// `SCENE=anim` turns its chain by hand: joint `i` at `sin(3t + 0.1 i)`
/// times 0.05 radians, `t` seconds in (a frame a sixtieth: its `0.05` a
/// frame), baked at 30 keys a second over one period. No mesh: skin to
/// its nodes.
pub fn chain(joints: usize) -> Vec<u8> {
    let joints = joints.max(1);
    let period = std::f32::consts::TAU / 3.;
    let keys = (period * 30.).ceil() as usize + 1;
    let times: Vec<f32> = (0..keys).map(|k| (k as f32 / 30.).min(period)).collect();
    let mut bin: Vec<u8> = Vec::new();
    let floats = |v: &[f32]| v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut push = |bytes: Vec<u8>, count: usize, kind: &str| {
        views.push(format!(
            r#"{{"buffer":0,"byteOffset":{},"byteLength":{}}}"#,
            bin.len(),
            bytes.len()
        ));
        bin.extend(bytes);
        accessors.push(format!(
            r#"{{"bufferView":{},"componentType":5126,"count":{count},"type":"{kind}"}}"#,
            views.len() - 1
        ));
        accessors.len() - 1
    };
    let input = push(floats(&times), keys, "SCALAR");
    let (mut samplers, mut channels) = (Vec::new(), Vec::new());
    for i in 0..joints {
        let turns: Vec<f32> = times
            .iter()
            .flat_map(|t| {
                let angle = (3. * t + i as f32 * 0.1).sin() * 0.05;
                let (s, c) = (angle / 2.).sin_cos();
                [0., 0., s, c]
            })
            .collect();
        let output = push(floats(&turns), keys, "VEC4");
        samplers.push(format!(r#"{{"input":{input},"output":{output}}}"#));
        channels.push(format!(
            r#"{{"sampler":{},"target":{{"node":{i},"path":"rotation"}}}}"#,
            samplers.len() - 1
        ));
    }
    let nodes: Vec<String> = (0..joints)
        .map(|i| {
            let child = if i + 1 < joints {
                format!(r#","children":[{}]"#, i + 1)
            } else {
                String::new()
            };
            format!(r#"{{"translation":[0,0.1,0]{child}}}"#)
        })
        .collect();
    format!(
        concat!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{}],"#,
            r#""animations":[{{"name":"Sway","samplers":[{}],"channels":[{}]}}],"#,
            r#""accessors":[{}],"bufferViews":[{}],"#,
            r#""buffers":[{{"byteLength":{},"uri":"data:application/octet-stream;base64,{}"}}]}}"#
        ),
        nodes.join(","),
        samplers.join(","),
        channels.join(","),
        accessors.join(","),
        views.join(","),
        bin.len(),
        base64(&bin),
    )
    .into_bytes()
}
