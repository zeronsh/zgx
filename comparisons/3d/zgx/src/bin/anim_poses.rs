//! zgx's poses against Blender's (`comparisons/3d/README.md`, "Animation"):
//! the joint positions `blender-anim/poses.py` wrote for a model's two clips
//! (`BOTTOM` under `TOP` at a weight, at a time), posed here by an animator
//! the same way (`TOP` on layer 1 at that weight over `BOTTOM` on layer 0),
//! and how far apart each case's joints are. Usage:
//! `anim_poses MODEL BOTTOM TOP BLENDER.json`.
use std::time::Instant;
use zgx_3d::*;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [model, bottom, top, blender] = &args[..] else {
        panic!("anim_poses MODEL BOTTOM TOP BLENDER.json");
    };
    let model = Model::from_bytes(&std::fs::read(model).expect("the model")).expect("a model");
    let (bottom, top) = (
        model.clip(bottom).expect("the bottom clip").clone(),
        model.clip(top).expect("the top clip").clone(),
    );
    let theirs: serde::Value = serde::from(&std::fs::read_to_string(blender).expect("Blender's"));
    let mut worst: f32 = 0.;
    let mut scale: f32 = 0.;
    for case in theirs.cases {
        let world = World::new();
        let placed = world.place_model(&model);
        let animator = world.add_animator(&placed);
        world
            .play(animator, &bottom, Play::new().from(case.time))
            .unwrap();
        world
            .set_layer(animator, 1, Layer::new().weight(case.weight))
            .unwrap();
        world
            .play(animator, &top, Play::new().layer(1).from(case.time))
            .unwrap();
        // One frame: a clip's first poses its start (here, `time`).
        world.animate(Instant::now());
        let mut far: f32 = 0.;
        for (name, [x, y, z]) in &case.joints {
            let node = model.node(name).unwrap_or_else(|| panic!("no node {name}"));
            let Some(matrix) = placed.node(node).and_then(|id| world.world_matrix(id)) else {
                continue;
            };
            let [ox, oy, oz, _] = matrix.columns[3];
            let ours = vec3(ox, oy, oz);
            let apart = (ours - vec3(*x, *y, *z)).length();
            if std::env::var_os("DETAIL").is_some() && apart > 1e-2 {
                println!("  {name}: {apart:.4}");
            }
            far = far.max(apart);
            scale = scale.max(vec3(*x, *y, *z).length());
        }
        println!(
            "time {:.2} weight {:.2}: joints at most {far:.5} apart",
            case.time, case.weight
        );
        worst = worst.max(far);
    }
    println!(
        "worst {worst:.5} (the model {scale:.1} across from its origin: {:.4}%)",
        worst / scale * 100.
    );
}

/// Just enough JSON for `poses.py`'s output.
mod serde {
    pub struct Value {
        pub cases: Vec<Case>,
    }
    pub struct Case {
        pub time: f32,
        pub weight: f32,
        pub joints: Vec<(String, [f32; 3])>,
    }
    pub fn from(text: &str) -> Value {
        let cases = text
            .split("{\"time\": ")
            .skip(1)
            .map(|case| {
                let number = |key: &str| -> f32 {
                    let at = case.find(key).unwrap() + key.len();
                    let rest = &case[at..];
                    let end = rest.find([',', '}']).unwrap();
                    rest[..end].trim().parse().unwrap()
                };
                let time = case[..case.find(',').unwrap()].trim().parse().unwrap();
                let weight = number("\"weight\": ");
                let joints_at = case.find("\"joints\": {").unwrap() + "\"joints\": {".len();
                let joints = case[joints_at..]
                    .split("], ")
                    .filter_map(|entry| {
                        let (name, values) = entry.split_once(": [")?;
                        let name = name.trim().trim_matches('"').to_owned();
                        let values: Vec<f32> = values
                            .trim_end_matches(['}', ']', ' '])
                            .split(',')
                            .filter_map(|v| v.trim().trim_end_matches([']', '}']).parse().ok())
                            .collect();
                        Some((name, [values[0], values[1], values[2]]))
                    })
                    .collect();
                Case {
                    time,
                    weight,
                    joints,
                }
            })
            .collect();
        Value { cases }
    }
}
