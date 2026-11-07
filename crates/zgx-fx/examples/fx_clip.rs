//! Writes a short clip of effects as raw RGBA frames to stdout, offscreen, no
//! window: `SCENE` is `sheet` (every effect and preset) or a highlight (`lens`,
//! `shockwave`, `heat_haze`, `glow`, `fire`, `fluted_glass`, `liquid_metal`,
//! `melt`, `wave`, `rim_lens`), over content moving under it. `SECONDS` (4),
//! `FPS` (30). Prints its size on stderr. `scripts/fx_clips.sh` encodes them.
//! Run: SCENE=lens cargo run --release -p zgx-fx --example fx_clip > frames.rgba
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};
use zgx::decoration::{Background, Decoration, FillShader, ShaderUniforms};
use zgx::scene::*;
use zgx_fx::*;
use zgx_gpu::GpuRenderer;

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
const COLORS: [Color; 5] = [
    Color(38, 48, 88, 255),
    Color(214, 92, 72, 255),
    Color(240, 196, 92, 255),
    Color(58, 140, 150, 255),
    Color(226, 228, 233, 255),
];
/// Rows of blocks and words, as a page has, for `moving` to scroll.
fn page(scene: &mut Scene, parent: NodeId, width: f32, rows: usize) -> NodeId {
    let moving = scene.append(
        parent,
        NodeKind::Container(Layout::Overlay),
        fixed(width, rows as f32 * 36.),
    );
    for row in 0..rows {
        let y = row as f32 * 36.;
        let band = scene.append(
            moving,
            NodeKind::Rect(COLORS[row % COLORS.len()]),
            fixed(width, 36.),
        );
        scene.set_transform(band, Transform { x: 0., y });
        let word = scene.append(
            moving,
            NodeKind::Text {
                text: "the quick brown fox jumps over the lazy dog".into(),
                color: Color(16, 18, 26, 255),
                font_size: 20.,
            },
            Style::default(),
        );
        scene.set_transform(
            word,
            Transform {
                x: 12. + (row % 3) as f32 * 40.,
                y: y + 6.,
            },
        );
    }
    moving
}
fn panel(
    scene: &mut Scene,
    parent: NodeId,
    fx: Option<&Fx>,
    at: (f32, f32),
    size: (f32, f32),
) -> NodeId {
    let filled = fx.is_some_and(|fx| fx.kind() == FxKind::Fill);
    let quad = QuadStyle {
        radius: 22.,
        fill: Color(24, 28, 40, if filled { 255 } else { 0 }),
        decoration: fx.filter(|_| filled).map(|fx| {
            Arc::new(Decoration {
                background: Some(Background::Shader(fx.shader())),
                ..Default::default()
            })
        }),
        ..Default::default()
    };
    let id = scene.append(
        parent,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad,
        },
        fixed(size.0, size.1),
    );
    scene.set_transform(id, Transform { x: at.0, y: at.1 });
    id
}
fn label(scene: &mut Scene, parent: NodeId, text: &str, at: (f32, f32), size: f32) {
    let word = scene.append(
        parent,
        NodeKind::Text {
            text: text.into(),
            color: Color(255, 255, 255, 255),
            font_size: size,
        },
        Style::default(),
    );
    scene.set_transform(word, Transform { x: at.0, y: at.1 });
}
/// The gallery's melt: its own WGSL, as that script writes it.
fn melt_source() -> String {
    let gallery = include_str!("gallery/app.luau");
    let start =
        gallery.find("ui.layer_shader([[").expect("its shader") + "ui.layer_shader([[".len();
    let end = start + gallery[start..].find("]])").expect("its end");
    gallery[start..end].to_owned()
}

fn main() {
    let name = std::env::var("SCENE").unwrap_or_else(|_| "lens".into());
    let seconds: f32 = env("SECONDS", 4.);
    let fps: f32 = env("FPS", 30.);
    let scale: f32 = env("SCALE", 2.);
    let start = Instant::now();
    let sheet = name == "sheet";
    let all: Vec<(String, Fx)> = if sheet { catalog() } else { Vec::new() };
    const CELL: (f32, f32) = (230., 150.);
    const COLUMNS: usize = 7;
    let size = match sheet {
        true => (
            COLUMNS as f32 * CELL.0,
            all.len().div_ceil(COLUMNS) as f32 * CELL.1,
        ),
        false => (640., 400.),
    };
    let (width, height) = ((size.0 * scale) as u32, (size.1 * scale) as u32);
    eprintln!("{width}x{height}");
    let mut gpu = GpuRenderer::new(width, height).expect("a renderer");
    gpu.set_scale_factor(scale);
    gpu.set_background(Color(12, 14, 20, 255));
    let mut scene = Scene::new(size.0, size.1);
    let fonts = gpu.text_system();
    scene.set_text_measurer(move |text: &str, size: f32, width: Option<f32>| {
        zgx_gpu::measure_text(&mut fonts.borrow_mut(), text, size, width)
    });
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    let moving = page(&mut scene, root, size.0, 80);
    // What each shows, on which node.
    let mut shown: Vec<(NodeId, Fx)> = Vec::new();
    let mut melt = None;
    let single = |name: &str| -> Fx {
        match name {
            "lens" => Lens::frosted().into(),
            "shockwave" => Shockwave {
                reach: 460.,
                amount: 26.,
                ..Default::default()
            }
            .into(),
            "heat_haze" => HeatHaze {
                amount: 12.,
                ..Default::default()
            }
            .into(),
            "glow" => Glow::aura().into(),
            "fire" => Fire::flames().into(),
            "fluted_glass" => FlutedGlass::reeded().into(),
            "liquid_metal" => LiquidMetal::gold().into(),
            "wave" => Wave::default().into(),
            "rim_lens" => RimLens::thick().into(),
            other => panic!("no scene {other}"),
        }
    };
    if sheet {
        for (index, (title, fx)) in all.iter().enumerate() {
            let at = (
                (index % COLUMNS) as f32 * CELL.0 + 30.,
                (index / COLUMNS) as f32 * CELL.1 + 18.,
            );
            let id = panel(&mut scene, root, Some(fx), at, (170., 90.));
            shown.push((id, fx.clone()));
            let strip = scene.append(
                root,
                NodeKind::Rect(Color(12, 14, 20, 255)),
                fixed(CELL.0, 22.),
            );
            scene.set_transform(
                strip,
                Transform {
                    x: at.0 - 30.,
                    y: at.1 + 102.,
                },
            );
            label(&mut scene, root, title, (at.0 - 22., at.1 + 104.), 13.);
        }
    } else if name == "melt" {
        let id = panel(&mut scene, root, None, (170., 60.), (300., 180.));
        scene.append(
            id,
            NodeKind::Rect(Color(48, 52, 60, 255)),
            fixed(300., 180.),
        );
        label(&mut scene, id, "Written in the script", (24., 70.), 24.);
        let shader = FillShader::new(melt_source(), None);
        scene.set_layer_shader(id, Some(shader));
        scene.set_layer_extent(id, 110., 0.);
        melt = Some(id);
    } else {
        let fx = single(&name);
        let (at, box_size) = match name.as_str() {
            "shockwave" => ((20., 20.), (600., 360.)),
            "wave" => ((60., 40.), (520., 320.)),
            "fire" => ((120., 170.), (400., 170.)),
            _ => ((120., 90.), (400., 220.)),
        };
        let id = panel(&mut scene, root, Some(&fx), at, box_size);
        if fx.kind() == FxKind::Layer
            && !matches!(
                name.as_str(),
                "lens" | "rim_lens" | "fluted_glass" | "heat_haze" | "shockwave"
            )
        {
            scene.append(
                id,
                NodeKind::Rect(Color(36, 40, 54, 255)),
                fixed(box_size.0, box_size.1),
            );
            label(
                &mut scene,
                id,
                "Sample",
                (box_size.0 * 0.5 - 50., box_size.1 * 0.5 - 18.),
                30.,
            );
        }
        shown.push((id, fx));
    }
    let frames = (seconds * fps) as u32;
    let mut out = std::io::stdout().lock();
    let every = fps * 1.3;
    for frame in 0..frames {
        let time = frame as f32 / fps;
        let now = start + Duration::from_secs_f32(time);
        scene.set_transform(
            moving,
            Transform {
                x: 0.,
                y: -(time * 60.) % (36. * 25.),
            },
        );
        for (id, fx) in &shown {
            if fx.shots().is_some() && (frame as f32 % every) < 1. {
                let bounds = scene.bounds(*id);
                let x = bounds.width * (0.3 + 0.4 * ((frame as f32 / every) % 2.));
                fx.fire(x, bounds.height * 0.5);
            }
            fx.apply(&mut scene, *id, time, false, now);
        }
        if let Some(id) = melt {
            scene.set_layer_uniforms(
                id,
                ShaderUniforms {
                    time,
                    params: [[70., 0., 0., 0.], [0.; 4], [0.; 4], [0.; 4]],
                },
            );
        }
        let report = scene.flush();
        gpu.render(&scene, &report.damage).expect("a frame");
        let pixels = gpu.readback().expect("its pixels");
        out.write_all(&pixels).expect("stdout");
    }
}
