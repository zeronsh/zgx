//! Custom materials (`shaders`): a mesh's shader shades, moves and cuts it
//! out, its parameters reach it, surface layers run over every lit mesh,
//! a shader reading time keeps the world drawing, and a shader's errors
//! come back at its own lines.
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_3d::*;
use zgx_gpu::GpuRenderer;

struct Window {
    ui: Ui,
    gpu: GpuRenderer,
    world: World,
}
const SIZE: f32 = 100.;
impl Window {
    fn new(world: &World) -> Option<Self> {
        let gpu = match GpuRenderer::new(SIZE as u32, SIZE as u32) {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
                panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
            }
            Err(_) => return None,
        };
        let mut ui = Ui::new(SIZE, SIZE);
        ui.mount(
            div().size(SIZE, SIZE).bg(Color(0, 0, 0, 255)).child(
                gpu_view("World", world.view())
                    .absolute()
                    .size(SIZE, SIZE)
                    .bg(Color(0, 128, 0, 255)),
            ),
        );
        Some(Self {
            ui,
            gpu,
            world: world.clone(),
        })
    }
    /// A frame, and more while a custom material's pipeline compiles (off
    /// the frame) until it's drawn by it: what a window's frame clock
    /// would draw.
    fn draw(&mut self) -> zgx_gpu::GpuStats {
        let stats = self.frame();
        let start = std::time::Instant::now();
        while self.world.compiling() && start.elapsed() < std::time::Duration::from_secs(20) {
            std::thread::sleep(std::time::Duration::from_millis(2));
            self.world.view().invalidate();
            self.frame();
        }
        stats
    }
    fn frame(&mut self) -> zgx_gpu::GpuStats {
        self.ui.prepare_frame();
        let report = self.ui.scene.borrow_mut().flush();
        self.gpu
            .render(&self.ui.scene.borrow(), &report.damage)
            .unwrap()
    }
    fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let pixels = self.gpu.readback().unwrap();
        let at = (y * SIZE as usize + x) * 4;
        pixels[at..at + 4].try_into().unwrap()
    }
}
fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 2)
}
fn unlit(rgb: u32) -> Material {
    let mut material = Material::rgb(rgb);
    material.unlit = true;
    material
}
/// A cube 3 in front of the camera, and the world.
fn cube(material: Material) -> (World, ObjectId) {
    let world = World::new();
    let id = world.add(Object::mesh(Geometry::cuboid(1., 1., 1.), material));
    world.set_camera(Camera::looking_at([0., 0., 3.], [0., 0., 0.]));
    (world, id)
}
const TINT: &str = "#zgx material 1
fn surface(s: SurfaceIn) -> Surface {
    var m = surface_default(s);
    m.base_color = params.tint.rgb;
    return m;
}";

#[test]
fn a_shaders_surface_colours_its_mesh_and_its_parameters_reach_it() {
    let (world, mesh) = cube(unlit(0x336699));
    let tint = world
        .add_shader(MaterialShader::new(TINT).param("tint", Param::Color))
        .unwrap();
    world.set_param(tint, "tint", [1., 0., 0., 1.]).unwrap();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    assert!(
        near(window.pixel(50, 50), [0x33, 0x66, 0x99, 255]),
        "no shader yet"
    );
    world.set_shader(mesh, Some(tint));
    assert_eq!(world.shader(mesh), Some(tint));
    window.draw();
    assert!(
        near(window.pixel(50, 50), [255, 0, 0, 255]),
        "{:?}",
        window.pixel(50, 50)
    );
    world.set_param(tint, "tint", [0., 0., 1., 1.]).unwrap();
    let frames = world.frames_drawn();
    window.draw();
    assert_eq!(world.frames_drawn(), frames + 1);
    assert!(
        near(window.pixel(50, 50), [0, 0, 255, 255]),
        "{:?}",
        window.pixel(50, 50)
    );
    // The same value again: nothing to draw.
    world.set_param(tint, "tint", [0., 0., 1., 1.]).unwrap();
    window.draw();
    assert_eq!(world.frames_drawn(), frames + 1);
    world.set_shader(mesh, None);
    window.draw();
    assert!(
        near(window.pixel(50, 50), [0x33, 0x66, 0x99, 255]),
        "its material again"
    );
    // Wrong names and kinds are errors, not panics.
    assert!(world.set_param(tint, "tnit", [0.; 4]).is_err());
    assert!(world.set_param(tint, "tint", 1.).is_err());
}

#[test]
fn meshes_without_a_shader_draw_as_they_did_beside_one_with() {
    let world = World::new();
    world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
    let left =
        world.add(Object::mesh(Geometry::cuboid(1., 1., 1.), unlit(0x336699)).at([-1., 0., 0.]));
    let right =
        world.add(Object::mesh(Geometry::cuboid(1., 1., 1.), unlit(0x336699)).at([1., 0., 0.]));
    let tint = world
        .add_shader(MaterialShader::new(TINT).param("tint", Param::Color))
        .unwrap();
    world.set_param(tint, "tint", [1., 1., 0., 1.]).unwrap();
    world.set_shader(right, Some(tint));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    let _ = left;
    assert!(
        near(window.pixel(25, 50), [0x33, 0x66, 0x99, 255]),
        "{:?}",
        window.pixel(25, 50)
    );
    assert!(
        near(window.pixel(75, 50), [255, 255, 0, 255]),
        "{:?}",
        window.pixel(75, 50)
    );
}

#[test]
fn displace_moves_a_meshs_vertices() {
    let (world, mesh) = cube(unlit(0xffffff));
    let away = world
        .add_shader(
            MaterialShader::new(
                "#zgx material 1
                 fn displace(v: VertexIn) -> Displaced {
                     var d: Displaced;
                     d.position = v.position + vec3(params.shift, 0.0, 0.0);
                     d.normal = v.normal;
                     return d;
                 }",
            )
            .param("shift", Param::Float),
        )
        .unwrap();
    world.set_shader(mesh, Some(away));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    assert!(near(window.pixel(50, 50), [255; 4]), "not moved yet");
    world.set_param(away, "shift", 0.8).unwrap();
    window.draw();
    assert_eq!(window.pixel(30, 50), [0, 128, 0, 255], "moved off its left");
    assert!(
        near(window.pixel(70, 50), [255; 4]),
        "{:?}",
        window.pixel(70, 50)
    );
}

#[test]
fn opacity_cuts_a_mesh_out() {
    let (world, mesh) = cube(unlit(0xffffff));
    let half = world
        .add_shader(MaterialShader::new(
            "#zgx material 1
             fn opacity(s: SurfaceIn) -> f32 {
                 return select(0.0, 1.0, s.world_position.x > 0.0);
             }",
        ))
        .unwrap();
    world.set_shader(mesh, Some(half));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    assert_eq!(window.pixel(40, 50), [0, 128, 0, 255], "cut out");
    assert!(
        near(window.pixel(60, 50), [255; 4]),
        "{:?}",
        window.pixel(60, 50)
    );
}

#[test]
fn a_texture_parameter_is_sampled() {
    let (world, mesh) = cube(unlit(0xffffff));
    let picture = world
        .add_shader(
            MaterialShader::new(
                "#zgx material 1
                 fn surface(s: SurfaceIn) -> Surface {
                     var m = surface_default(s);
                     m.base_color = textureSample(params.picture, material_sampler, s.uv).rgb;
                     return m;
                 }",
            )
            .param("picture", Param::Texture),
        )
        .unwrap();
    world.set_shader(mesh, Some(picture));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    assert!(near(window.pixel(50, 50), [255; 4]), "white until set");
    let red = Texture::from_rgba8(2, 2, [255, 0, 0, 255].repeat(4)).unwrap();
    world.set_param(picture, "picture", &red).unwrap();
    window.draw();
    assert!(
        near(window.pixel(50, 50), [255, 0, 0, 255]),
        "{:?}",
        window.pixel(50, 50)
    );
}

#[test]
fn a_surface_layer_runs_over_every_lit_mesh() {
    let world = World::new();
    world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
    world.add_light(Light::Hemisphere {
        sky: [1.; 3],
        ground: [1.; 3],
        intensity: 1.,
    });
    world
        .add(Object::mesh(Geometry::cuboid(1., 1., 1.), Material::rgb(0x808080)).at([-1., 0., 0.]));
    world.add(Object::mesh(Geometry::cuboid(1., 1., 1.), unlit(0x808080)).at([1., 0., 0.]));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    let (lit, flat) = (window.pixel(25, 50), window.pixel(75, 50));
    let wet = world
        .add_surface_layer(
            MaterialShader::new(
                "#zgx material 1
                 fn surface(s: SurfaceIn) -> Surface {
                     var m = surface_default(s);
                     m.base_color *= params.darken;
                     return m;
                 }",
            )
            .param("darken", Param::Float),
        )
        .unwrap();
    world.set_param(wet, "darken", 0.25).unwrap();
    window.draw();
    assert!(
        window.pixel(25, 50)[0] + 30 < lit[0],
        "{:?} against {lit:?}",
        window.pixel(25, 50)
    );
    assert_eq!(window.pixel(75, 50), flat, "unlit: as it was");
    world.remove_surface_layer(wet);
    window.draw();
    assert_eq!(window.pixel(25, 50), lit, "as before the layer");
}

#[test]
fn a_shader_reads_the_scene_behind_it() {
    let world = World::new();
    world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
    world.add(Object::mesh(Geometry::cuboid(3., 3., 0.2), unlit(0xff0000)).at([0., 0., -1.]));
    let pane = world.add(Object::mesh(Geometry::cuboid(1., 1., 0.2), unlit(0x0000ff)));
    let see = world
        .add_shader(
            MaterialShader::new(
                "#zgx material 1
                 fn surface(s: SurfaceIn) -> Surface {
                     var m = surface_default(s);
                     m.base_color = scene_color(s.screen_uv, 0.0);
                     return m;
                 }",
            )
            .reads_scene(),
        )
        .unwrap();
    world.set_shader(pane, Some(see));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    assert!(
        near(window.pixel(50, 50), [255, 0, 0, 255]),
        "{:?}",
        window.pixel(50, 50)
    );
}

#[test]
fn a_shader_reading_time_keeps_its_world_drawing_while_its_mesh_is_in_view() {
    let (world, mesh) = cube(unlit(0xffffff));
    let pulse = world
        .add_shader(
            MaterialShader::new(
                "#zgx material 1
                 fn surface(s: SurfaceIn) -> Surface {
                     var m = surface_default(s);
                     m.base_color *= 0.5 + 0.5 * sin(frame.time);
                     return m;
                 }",
            )
            .animated(),
        )
        .unwrap();
    // (This window has no frame clock: whether the view animates is what
    // says the world keeps drawing.)
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    assert!(!world.view().animating(), "no shader: at rest");
    world.set_shader(mesh, Some(pulse));
    window.draw();
    assert!(world.view().animating(), "it animates");
    // Out of view: at rest again, once culling looks. (It rests for up to
    // 15 frames after frames that left nothing out, and a frame where only
    // the view changed culls by the boxes the last change made: a change to
    // the meshes after the rest makes them.)
    world.set_transform(mesh, Transform::at([50., 0., 0.]));
    for step in 0..16 {
        world.set_camera(Camera::looking_at(
            [0., 0., 3. + step as f32 * 0.01],
            [0., 0., 0.],
        ));
        window.draw();
    }
    world.set_material(mesh, unlit(0xfefefe));
    window.draw();
    assert!(!world.view().animating(), "out of view: at rest");
    world.set_transform(mesh, Transform::default());
    window.draw();
    assert!(world.view().animating(), "back in view");
    world.set_shader(mesh, None);
    window.draw();
    assert!(!world.view().animating(), "its shader gone");
}

fn error_of(shader: MaterialShader) -> ShaderError {
    World::new().add_shader(shader).unwrap_err()
}

#[test]
fn a_shaders_errors_are_at_its_own_lines() {
    let at = |source: &str| {
        let e = error_of(MaterialShader::new(source));
        (e.line, e.column, e.message)
    };
    let (line, _, message) =
        at("#zgx material 2\nfn surface(s: SurfaceIn) -> Surface { return surface_default(s); }");
    assert_eq!(line, 1, "{message}");
    // A syntax error on its third line.
    let (line, _, message) = at(
        "#zgx material 1\n\nfn surface(s: SurfaceIn) -> Surface { return surface_default(s) }\n",
    );
    assert_eq!(line, 3, "{message}");
    // A name it never declared, at its column.
    let (line, column, message) = at(
        "#zgx material 1\nfn surface(s: SurfaceIn) -> Surface {\n    var m = surface_default(s);\n    m.roughness = wobble;\n    return m;\n}\n",
    );
    assert_eq!((line, column), (4, 19), "{message}");
    assert!(message.contains("wobble"), "{message}");
    // A function by the engine's name with another signature.
    let (line, _, message) =
        at("#zgx material 1\n\nfn surface(s: SurfaceIn) -> f32 { return 1.0; }\n");
    assert_eq!(line, 3, "{message}");
    assert!(
        message.contains("fn surface(s: SurfaceIn) -> Surface"),
        "{message}"
    );
    // What it must say it reads.
    let (line, _, message) = at(
        "#zgx material 1\nfn surface(s: SurfaceIn) -> Surface {\n    var m = surface_default(s);\n    m.roughness = frame.time;\n    return m;\n}\n",
    );
    assert_eq!(line, 4, "{message}");
    assert!(message.contains("animated"), "{message}");
    let (line, _, message) = at(
        "#zgx material 1\nfn surface(s: SurfaceIn) -> Surface {\n    var m = surface_default(s);\n    m.base_color = scene_color(s.screen_uv, 0.0);\n    return m;\n}\n",
    );
    assert_eq!(line, 4, "{message}");
    assert!(message.contains("reads_scene"), "{message}");
    // A layer's parameters are numbers.
    let e = World::new()
        .add_surface_layer(
            MaterialShader::new("#zgx material 1\nfn surface(s: SurfaceIn) -> Surface { return surface_default(s); }")
                .param("mask", Param::Texture),
        )
        .unwrap_err();
    assert!(e.message.contains("numbers"), "{e}");
}

#[test]
fn a_shaders_own_names_never_meet_the_engines_or_another_shaders() {
    // `shade`, `encode` and `PI` are the engine's; `wave` is both shaders'.
    let source = "#zgx material 1
        const PI: f32 = 3.0;
        fn shade(x: f32) -> f32 { return x * PI; }
        fn wave(p: vec3<f32>) -> f32 { return shade(p.x); }
        fn surface(s: SurfaceIn) -> Surface {
            var m = surface_default(s);
            m.roughness = clamp(wave(s.world_position), 0.1, 1.0);
            return m;
        }";
    let world = World::new();
    let shader = world.add_shader(MaterialShader::new(source)).unwrap();
    world
        .add_surface_layer(MaterialShader::new(source))
        .unwrap();
    let mesh = world.add(Object::mesh(
        Geometry::cuboid(1., 1., 1.),
        Material::default(),
    ));
    world.set_shader(mesh, Some(shader));
    world.set_camera(Camera::looking_at([0., 0., 3.], [0., 0., 0.]));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    // A pipeline holding both compiles and draws.
    window.draw();
    assert_ne!(window.pixel(50, 50), [0, 128, 0, 255]);
}

/// Pixels of the floor darker than half the brightest (shadows, here).
fn dark(window: &Window) -> usize {
    let pixels = window.gpu.readback().unwrap();
    let brightest = pixels.chunks(4).map(|p| p[0]).max().unwrap_or(0);
    // (The box's green background past the floor, and the floor's edge
    // over it, aren't: grey alone is.)
    let floor = |p: &&[u8]| p[0].abs_diff(p[1]) < 16;
    pixels
        .chunks(4)
        .filter(floor)
        .filter(|p| p[0] < brightest / 2)
        .count()
}

#[test]
fn a_shaders_displace_and_opacity_move_and_cut_its_shadow() {
    let world = World::new();
    world.set_shadows(true);
    world.add_light(Light::Directional {
        direction: vec3(0.3, -1., 0.2),
        color: [1.; 3],
        intensity: 3.,
    });
    world.add_light(Light::Hemisphere {
        sky: [1.; 3],
        ground: [1.; 3],
        intensity: 0.1,
    });
    world.add(
        Object::mesh(Geometry::cuboid(8., 0.2, 8.), Material::rgb(0xc0c0c0)).at([0., -0.6, 0.]),
    );
    let caster = world
        .add(Object::mesh(Geometry::cuboid(1., 1., 1.), Material::rgb(0xc0c0c0)).at([0., 0.5, 0.]));
    world.set_camera(Camera::looking_at([0., 6., 3.], [0., 0., 0.]));
    let away = world
        .add_shader(MaterialShader::new(
            "#zgx material 1
             fn displace(v: VertexIn) -> Displaced {
                 var d: Displaced;
                 d.position = v.position + vec3(50.0, 0.0, 0.0);
                 d.normal = v.normal;
                 return d;
             }",
        ))
        .unwrap();
    let gone = world
        .add_shader(MaterialShader::new(
            "#zgx material 1
             fn opacity(s: SurfaceIn) -> f32 {
                 return 0.0;
             }",
        ))
        .unwrap();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    let shadowed = dark(&window);
    assert!(shadowed > 20, "its shadow: {shadowed} pixels");
    world.set_shader(caster, Some(away));
    window.draw();
    assert!(
        dark(&window) < 5,
        "moved away, its shadow too: {}",
        dark(&window)
    );
    world.set_shader(caster, Some(gone));
    window.draw();
    assert!(
        dark(&window) < 5,
        "cut out, its shadow too: {}",
        dark(&window)
    );
    world.set_shader(caster, None);
    window.draw();
    assert_eq!(dark(&window), shadowed, "as it was");
}

#[cfg(feature = "lighting")]
#[test]
fn a_custom_material_is_lit_through_clusters_as_any_mesh_is() {
    // The same lit cube, its shader doing nothing, with clusters and without:
    // the custom pipeline binds the clustered frame as the stock one does.
    let draw = |clustered: bool, shaded: bool| {
        let (world, mesh) = cube(Material::rgb(0xc0c0c0));
        world.set_clustered_lights(clustered);
        for i in 0..12 {
            world.add_light(Light::Point {
                position: vec3(i as f32 * 0.2 - 1.1, 0.5, 1.2),
                color: [1.; 3],
                intensity: 0.4,
                range: 3.,
            });
        }
        if shaded {
            let plain = world
                .add_shader(MaterialShader::new(
                    "#zgx material 1\nfn surface(s: SurfaceIn) -> Surface { return surface_default(s); }",
                ))
                .unwrap();
            world.set_shader(mesh, Some(plain));
        }
        let mut window = Window::new(&world)?;
        window.draw();
        Some(window.pixel(50, 50))
    };
    let Some(stock) = draw(true, false) else {
        return;
    };
    let custom = draw(true, true).unwrap();
    assert!(near(custom, stock), "{custom:?} against {stock:?}");
    assert_ne!(stock, [0, 128, 0, 255]);
}

#[test]
fn a_cut_out_or_skinned_mesh_with_a_shader_casts_its_shadow() {
    // A cut-out mesh's shadow binds its maps, which a shader's binding
    // holds with its own: it casts by its shader's pipeline. A skinned one
    // that only shades casts as its variant does.
    let world = World::new();
    world.set_shadows(true);
    world.add_light(Light::Directional {
        direction: vec3(0.3, -1., 0.2),
        color: [1.; 3],
        intensity: 3.,
    });
    world.add(
        Object::mesh(Geometry::cuboid(8., 0.2, 8.), Material::rgb(0xc0c0c0)).at([0., -0.6, 0.]),
    );
    let card = world.add(
        Object::mesh(Geometry::cuboid(1., 1., 1.), Material::rgb(0xc0c0c0)).at([-1., 0.5, 0.]),
    );
    let solid = Texture::from_rgba8(1, 1, vec![255; 4]).unwrap();
    world.set_texture(card, Some(solid));
    world.set_alpha_cutoff(card, Some(0.5));
    let joint = world.add(Object::group().at([1., 0.5, 0.]));
    let shape = Geometry::cuboid(1., 1., 1.);
    let vertices = shape.vertices().len();
    let skinned = shape
        .with_skin(vec![[0; 4]; vertices], vec![[1., 0., 0., 0.]; vertices])
        .unwrap();
    let body = world.add(Object::mesh(skinned, Material::rgb(0xc0c0c0)));
    world.set_skin(
        body,
        Some(Skin::new(vec![joint], vec![Mat4::IDENTITY]).unwrap()),
    );
    world.set_camera(Camera::looking_at([0., 6., 3.], [0., 0., 0.]));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    let shadowed = dark(&window);
    let same = world
        .add_shader(MaterialShader::new(
            "#zgx material 1\nfn surface(s: SurfaceIn) -> Surface { return surface_default(s); }",
        ))
        .unwrap();
    world.set_shader(card, Some(same));
    world.set_shader(body, Some(same));
    window.draw();
    assert_eq!(dark(&window), shadowed, "the same shadows");
}

#[cfg(feature = "lighting")]
#[test]
fn a_cut_out_custom_mesh_in_cascades_casts_whole() {
    // Cascades cast a custom material's mesh by its stock variant, whole
    // (they don't run its shader yet): drawn, and without error.
    let world = World::new();
    world.set_shadows(true);
    world.set_cascades(Some(Cascades::new(2)));
    world.add_light(Light::Directional {
        direction: vec3(0.3, -1., 0.2),
        color: [1.; 3],
        intensity: 3.,
    });
    world.add(
        Object::mesh(Geometry::cuboid(8., 0.2, 8.), Material::rgb(0xc0c0c0)).at([0., -0.6, 0.]),
    );
    let card = world
        .add(Object::mesh(Geometry::cuboid(1., 1., 1.), Material::rgb(0xc0c0c0)).at([0., 0.5, 0.]));
    world.set_texture(card, Some(Texture::from_rgba8(1, 1, vec![255; 4]).unwrap()));
    world.set_alpha_cutoff(card, Some(0.5));
    let same = world
        .add_shader(MaterialShader::new(
            "#zgx material 1\nfn opacity(s: SurfaceIn) -> f32 { return 1.0; }",
        ))
        .unwrap();
    world.set_shader(card, Some(same));
    world.set_camera(Camera::looking_at([0., 6., 3.], [0., 0., 0.]));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    assert!(dark(&window) > 20, "its shadow: {}", dark(&window));
}

#[test]
fn what_an_app_hands_a_world_is_bounded() {
    let world = World::new();
    let body = "fn surface(s: SurfaceIn) -> Surface { return surface_default(s); }";
    let long = format!("#zgx material 1\n{body}\n{}", "// padding\n".repeat(30_000));
    assert!(
        world
            .add_shader(MaterialShader::new(long))
            .unwrap_err()
            .message
            .contains("at most")
    );
    let mut many = MaterialShader::new(format!("#zgx material 1\n{body}"));
    for i in 0..65 {
        many = many.param(&format!("p{i}"), Param::Float);
    }
    assert!(
        world
            .add_shader(many)
            .unwrap_err()
            .message
            .contains("at most")
    );
    let named = MaterialShader::new(format!("#zgx material 1\n{body}"))
        .param(&"n".repeat(65), Param::Float);
    assert!(world.add_shader(named).is_err());
    for _ in 0..8 {
        world
            .add_surface_layer(MaterialShader::new(format!("#zgx material 1\n{body}")))
            .unwrap();
    }
    let ninth = world.add_surface_layer(MaterialShader::new(format!("#zgx material 1\n{body}")));
    assert!(ninth.unwrap_err().message.contains("no more"));
}

#[test]
fn what_the_device_refuses_draws_the_mesh_without_its_shader_and_says_why() {
    // A mesh binding more textures than a stage may (its maps, glass's and
    // a coat's, four of its shader's and the scene behind): no frame stops.
    // (A texture past a device's side can't be made: `Texture::from_rgba8`
    // refuses past 8192, and a binding checks the device's own limit.)
    let (world, mesh) = cube(unlit(0xffffff));
    world.set_shadows(true);
    let picture = world
        .add_shader(
            MaterialShader::new(
                "#zgx material 1
                 fn surface(s: SurfaceIn) -> Surface {
                     var m = surface_default(s);
                     m.base_color = textureSample(params.picture, material_sampler, s.uv).rgb;
                     return m;
                 }",
            )
            .param("picture", Param::Texture),
        )
        .unwrap();
    world.set_shader(mesh, Some(picture));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    assert_eq!(world.shader_error(picture), None);

    let crowded = world
        .add_shader(
            MaterialShader::new(
                "#zgx material 1
                 fn surface(s: SurfaceIn) -> Surface {
                     var m = surface_default(s);
                     let a = textureSample(params.a, material_sampler, s.uv).rgb;
                     let b = textureSample(params.b, material_sampler, s.uv).rgb;
                     let c = textureSample(params.c, material_sampler, s.uv).rgb;
                     let d = textureSample(params.d, material_sampler, s.uv).rgb;
                     m.base_color = (a + b + c + d) * 0.25 * scene_color(s.screen_uv, 0.0);
                     return m;
                 }",
            )
            .param("a", Param::Texture)
            .param("b", Param::Texture)
            .param("c", Param::Texture)
            .param("d", Param::Texture)
            .reads_scene(),
        )
        .unwrap();
    world.set_material(mesh, Material::rgb(0xffffff));
    world.set_texture(mesh, Some(Texture::from_rgba8(1, 1, vec![255; 4]).unwrap()));
    world.set_transmission(mesh, Some(Transmission::new(0.5)));
    world.set_clearcoat(mesh, Some(Clearcoat::new(1.)));
    world.set_shader(mesh, Some(crowded));
    window.draw();
    window.draw();
    // Either this device binds that many, or it refused and said so.
    if let Some(said) = world.shader_error(crowded) {
        assert!(!said.is_empty());
    }
    assert_ne!(
        window.pixel(50, 50),
        [0, 128, 0, 255],
        "the mesh still draws"
    );
}

/// What a world draws when it reached its state the long way (drawn, then
/// changed by `after`, drawn, the camera moved away and back, drawn) and
/// when it was made in it: equal, within a level a channel.
/// A world `retained_matches_fresh` draws in: a sun casting shadows over
/// a floor.
fn shadowed(world: &World) {
    world.set_shadows(true);
    world.add_light(Light::Directional {
        direction: vec3(0.3, -1., 0.2),
        color: [1.; 3],
        intensity: 3.,
    });
    world.add_light(Light::Hemisphere {
        sky: [1.; 3],
        ground: [1.; 3],
        intensity: 0.2,
    });
    world.add(
        Object::mesh(Geometry::cuboid(8., 0.2, 8.), Material::rgb(0xc0c0c0)).at([0., -0.6, 0.]),
    );
    world.set_camera(Camera::looking_at([0., 4., 4.], [0., 0., 0.]));
}
fn retained_matches_fresh(
    name: &str,
    setup: impl Fn(&World) -> (Vec<ShaderId>, ObjectId),
    after: impl Fn(&World, &[ShaderId], ObjectId),
) {
    let lit = shadowed;
    let fresh = World::new();
    lit(&fresh);
    let (ids, mesh) = setup(&fresh);
    after(&fresh, &ids, mesh);
    let Some(mut window) = Window::new(&fresh) else {
        return;
    };
    window.draw();
    let expected = window.gpu.readback().unwrap();

    let world = World::new();
    lit(&world);
    let (ids, mesh) = setup(&world);
    let mut window = Window::new(&world).unwrap();
    window.draw();
    after(&world, &ids, mesh);
    window.draw();
    world.set_camera(Camera::looking_at([0.5, 4., 4.], [0., 0., 0.]));
    window.draw();
    world.set_camera(Camera::looking_at([0., 4., 4.], [0., 0., 0.]));
    window.draw();
    let actual = window.gpu.readback().unwrap();
    let differ = actual
        .iter()
        .zip(&expected)
        .filter(|(a, b)| a.abs_diff(**b) > 1)
        .count();
    assert_eq!(
        differ, 0,
        "{name}: {differ} channels differ from a fresh world's"
    );
}
/// A shader tinting by `tint` and lifting its mesh by `lift`, on a cube
/// casting a shadow (and a second shader, plain).
fn tinted_and_lifted(world: &World) -> (Vec<ShaderId>, ObjectId) {
    let moving = world
        .add_shader(
            MaterialShader::new(
                "#zgx material 1
                 fn displace(v: VertexIn) -> Displaced {
                     var d: Displaced;
                     d.position = v.position + vec3(params.lift, 0.0, 0.0);
                     d.normal = v.normal;
                     return d;
                 }
                 fn surface(s: SurfaceIn) -> Surface {
                     var m = surface_default(s);
                     m.base_color *= params.tint.rgb;
                     return m;
                 }",
            )
            .param("tint", Param::Color)
            .param("lift", Param::Float),
        )
        .unwrap();
    world.set_param(moving, "tint", [1., 0.5, 0.5, 1.]).unwrap();
    let plain = world
        .add_shader(MaterialShader::new(TINT).param("tint", Param::Color))
        .unwrap();
    world.set_param(plain, "tint", [0.3, 0.9, 0.3, 1.]).unwrap();
    let cube = world
        .add(Object::mesh(Geometry::cuboid(1., 1., 1.), Material::rgb(0xffffff)).at([0., 0.5, 0.]));
    world.set_shader(cube, Some(moving));
    (vec![moving, plain], cube)
}

#[test]
fn a_retained_world_with_custom_materials_draws_what_a_fresh_one_does() {
    retained_matches_fresh("a colour", tinted_and_lifted, |world, ids, _| {
        world.set_param(ids[0], "tint", [0.2, 0.4, 1., 1.]).unwrap();
    });
    retained_matches_fresh(
        "a displacement, its shadow",
        tinted_and_lifted,
        |world, ids, _| {
            world.set_param(ids[0], "lift", 1.2).unwrap();
        },
    );
    retained_matches_fresh("another shader", tinted_and_lifted, |world, ids, mesh| {
        world.set_shader(mesh, Some(ids[1]));
    });
    retained_matches_fresh("no shader", tinted_and_lifted, |world, _, mesh| {
        world.set_shader(mesh, None);
    });
    let layer = |world: &World| {
        world
            .add_surface_layer(MaterialShader::new(
                "#zgx material 1\nfn surface(s: SurfaceIn) -> Surface { var m = surface_default(s); m.base_color *= 0.4; return m; }",
            ))
            .unwrap()
    };
    retained_matches_fresh("a layer added", tinted_and_lifted, |world, _, _| {
        layer(world);
    });
    retained_matches_fresh(
        "a layer removed",
        |world| {
            let (mut ids, mesh) = tinted_and_lifted(world);
            ids.push(layer(world));
            (ids, mesh)
        },
        |world, ids, _| world.remove_surface_layer(ids[2]),
    );
}

#[cfg(feature = "lighting")]
#[test]
fn a_retained_world_with_cascades_and_custom_materials_draws_what_a_fresh_one_does() {
    let cascaded = |world: &World| {
        world.set_cascades(Some(Cascades::new(2)));
        tinted_and_lifted(world)
    };
    retained_matches_fresh("a displacement, cascaded", cascaded, |world, ids, _| {
        world.set_param(ids[0], "lift", 1.2).unwrap();
    });
    retained_matches_fresh("another shader, cascaded", cascaded, |world, ids, mesh| {
        world.set_shader(mesh, Some(ids[1]));
    });
}

#[test]
fn a_mesh_its_shader_moves_into_view_is_drawn_while_culling() {
    // A cube 20 m right of the view, its shader moving every vertex 20 m
    // back: in view, culling or not (review finding CM1). Said or not how
    // far, it draws; said, it's still culled where the box grown by that is
    // out of view.
    for reach in [None, Some(20.5)] {
        let (world, mesh) = cube(unlit(0xffffff));
        world.set_transform(mesh, Transform::at([20., 0., 0.]));
        let mut shader = MaterialShader::new(
            "#zgx material 1
             fn displace(v: VertexIn) -> Displaced {
                 var d: Displaced;
                 d.position = v.position - vec3(20.0, 0.0, 0.0);
                 d.normal = v.normal;
                 return d;
             }",
        );
        if let Some(r) = reach {
            shader = shader.displaces_by(r);
        }
        let back = world.add_shader(shader).unwrap();
        world.set_shader(mesh, Some(back));
        let Some(mut window) = Window::new(&world) else {
            return;
        };
        window.draw();
        assert!(
            near(window.pixel(50, 50), [255; 4]),
            "{reach:?}: {:?}",
            window.pixel(50, 50)
        );
        assert_eq!(world.culled(), 0, "{reach:?}");
    }
    // Moving it 1 m and saying so: 20 m out, culled.
    let (world, mesh) = cube(unlit(0xffffff));
    world.set_transform(mesh, Transform::at([20., 0., 0.]));
    let nudge = world
        .add_shader(
            MaterialShader::new(
                "#zgx material 1
                 fn displace(v: VertexIn) -> Displaced {
                     var d: Displaced;
                     d.position = v.position + vec3(0.0, 1.0, 0.0);
                     d.normal = v.normal;
                     return d;
                 }",
            )
            .displaces_by(1.),
        )
        .unwrap();
    world.set_shader(mesh, Some(nudge));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    assert_eq!(world.culled(), 1);
    assert!(
        world
            .add_shader(MaterialShader::new(TINT).displaces_by(f32::NAN))
            .is_err()
    );
}

#[test]
fn time_read_another_way_still_needs_animated() {
    // The token check saw `frame.time` alone; `let f = frame; f.time` read
    // time unseen, and the shader froze (review finding CM4).
    let source = "#zgx material 1
fn surface(s: SurfaceIn) -> Surface {
    var m = surface_default(s);
    let f = frame;
    m.roughness = fract(f.time);
    return m;
}";
    let e = error_of(MaterialShader::new(source));
    assert_eq!(e.line, 5, "{e}");
    assert!(e.message.contains("animated"), "{e}");
    assert!(
        World::new()
            .add_shader(MaterialShader::new(source).animated())
            .is_ok()
    );
}

/// A PNG of `side` squared pixels of `rgba`.
#[cfg(feature = "gltf")]
fn png_of(side: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, side, side);
    encoder.set_color(png::ColorType::Rgba);
    let pixels = rgba.repeat((side * side) as usize);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
    bytes
}

// (PNG files decode with the `gltf` feature: zgx-gpu's `png`.)
#[cfg(feature = "gltf")]
#[test]
fn a_large_texture_parameter_loads_off_the_frame_and_then_shows() {
    // A texture parameter is decoded and put on the GPU as a mesh's maps
    // are, on the loading threads: the mesh draws with white in its place
    // meanwhile, and by it once it arrives (review addition to CM3).
    let (world, mesh) = cube(unlit(0xffffff));
    let picture = world
        .add_shader(
            MaterialShader::new(
                "#zgx material 1
                 fn surface(s: SurfaceIn) -> Surface {
                     var m = surface_default(s);
                     m.base_color = textureSample(params.picture, material_sampler, s.uv).rgb;
                     return m;
                 }",
            )
            .param("picture", Param::Texture),
        )
        .unwrap();
    world.set_shader(mesh, Some(picture));
    let red = Texture::decode(&png_of(512, [255, 0, 0, 255])).unwrap();
    world.set_param(picture, "picture", &red).unwrap();
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    // The frame first binding it asks for it and draws on.
    window.frame();
    assert!(world.loading(), "on its way, not decoded in the frame");
    window.draw();
    let start = std::time::Instant::now();
    while world.loading() && start.elapsed() < std::time::Duration::from_secs(20) {
        std::thread::sleep(std::time::Duration::from_millis(2));
        world.view().invalidate();
        window.frame();
    }
    assert!(!world.loading());
    window.draw();
    assert!(
        near(window.pixel(50, 50), [255, 0, 0, 255]),
        "{:?}",
        window.pixel(50, 50)
    );
}

/// A device of wgpu's no-op backend and its queue: pipelines made, nothing
/// drawn.
fn noop_device() -> (zgx_gpu::wgpu::Device, zgx_gpu::wgpu::Queue) {
    use zgx_gpu::wgpu;
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::NOOP;
    descriptor.backend_options.noop.enable = true;
    fn now<T>(future: impl std::future::Future<Output = T>) -> T {
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match std::pin::pin!(future).poll(&mut context) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("the no-op backend waited"),
        }
    }
    let adapter = now(wgpu::Instance::new(descriptor).request_adapter(&Default::default()));
    now(adapter.unwrap().request_device(&Default::default())).unwrap()
}

#[test]
fn a_shader_is_ready_only_while_a_renderer_holds_its_pipelines() {
    // `shader_ready` stayed true after a device change, before the new
    // device had compiled anything (review finding on 822fbb175).
    use zgx_gpu::{
        viewport::{Viewport, ViewportFrame},
        wgpu,
    };
    let (world, mesh) = cube(unlit(0xffffff));
    let tint = world
        .add_shader(MaterialShader::new(TINT).param("tint", Param::Color))
        .unwrap();
    world.set_shader(mesh, Some(tint));
    let (device, queue) = noop_device();
    let format = wgpu::TextureFormat::Bgra8Unorm;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut painter = world.viewport();
    let frames_until_ready = |painter: &mut dyn zgx_gpu::viewport::Viewport| {
        let start = std::time::Instant::now();
        loop {
            let mut encoder = device.create_command_encoder(&Default::default());
            let mut frame =
                ViewportFrame::new(&device, &queue, &mut encoder, &view, format, (64, 64), 1.);
            painter.render(&mut frame);
            queue.submit([encoder.finish()]);
            if world.shader_ready(tint) || start.elapsed() > std::time::Duration::from_secs(20) {
                return world.shader_ready(tint);
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    };
    assert!(!world.shader_ready(tint), "nothing compiled before a frame");
    assert!(frames_until_ready(&mut painter), "compiled by frames");
    painter.device_changed();
    assert!(
        !world.shader_ready(tint),
        "its device gone, its pipelines with it"
    );
    assert!(
        frames_until_ready(&mut painter),
        "compiled again on the new one"
    );
    drop(painter);
    assert!(!world.shader_ready(tint), "no renderer holds them");
}

#[test]
fn a_surface_layer_is_ready_once_a_renderer_draws_with_it() {
    // `shader_ready` stayed false for a surface layer that was drawing
    // (G11). A layer is ready as a shader is: while a renderer holds the
    // pipelines it runs in, the stock ones and its meshes' shaders' alike.
    use zgx_gpu::{
        viewport::{Viewport, ViewportFrame},
        wgpu,
    };
    let (world, mesh) = cube(unlit(0xffffff));
    let tint = world
        .add_shader(MaterialShader::new(TINT).param("tint", Param::Color))
        .unwrap();
    world.set_shader(mesh, Some(tint));
    world.add(Object::mesh(Geometry::cuboid(1., 1., 1.), Material::default()).at([1., 0., 0.]));
    let wet = world
        .add_surface_layer(MaterialShader::new(
            "#zgx material 1\nfn surface(s: SurfaceIn) -> Surface {\n    var m = surface_default(s);\n    m.roughness *= 0.5;\n    return m;\n}\n",
        ))
        .unwrap();
    let (device, queue) = noop_device();
    let format = wgpu::TextureFormat::Bgra8Unorm;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut painter = world.viewport();
    let frame = |painter: &mut dyn Viewport| {
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut frame =
            ViewportFrame::new(&device, &queue, &mut encoder, &view, format, (64, 64), 1.);
        painter.render(&mut frame);
        queue.submit([encoder.finish()]);
    };
    let frames_until_ready = |painter: &mut dyn Viewport| {
        let start = std::time::Instant::now();
        loop {
            frame(painter);
            if world.shader_ready(wet) || start.elapsed() > std::time::Duration::from_secs(20) {
                return world.shader_ready(wet);
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    };
    assert!(!world.shader_ready(wet), "nothing made before a frame");
    assert!(frames_until_ready(&mut painter), "made by frames");
    assert!(
        world.shader_ready(tint),
        "ready once its meshes' shaders are, made with it"
    );
    painter.device_changed();
    assert!(
        !world.shader_ready(wet),
        "its device gone, its pipelines with it"
    );
    assert!(
        frames_until_ready(&mut painter),
        "made again on the new one"
    );
    // Another layer added makes every pipeline again: still ready, both.
    let dry = world
        .add_surface_layer(MaterialShader::new(
            "#zgx material 1\nfn surface(s: SurfaceIn) -> Surface {\n    return surface_default(s);\n}\n",
        ))
        .unwrap();
    assert!(frames_until_ready(&mut painter));
    assert!(world.shader_ready(dry));
    // Taken off, it is held no more; the other is again once its meshes'
    // shaders are made anew without it.
    world.remove_surface_layer(dry);
    frame(&mut painter);
    assert!(!world.shader_ready(dry), "removed");
    assert!(frames_until_ready(&mut painter));
    drop(painter);
    assert!(!world.shader_ready(wet), "no renderer holds it");
}

#[cfg(feature = "compute")]
mod gpu_data {
    //! An app's GPU buffers and textures as material parameters (H3).
    use super::*;

    const FROM_BUFFER: &str = "#zgx material 1
fn surface(s: SurfaceIn) -> Surface {
    var m = surface_default(s);
    m.base_color = params.colors[0].rgb;
    return m;
}";
    fn coloured(world: &World) -> (ShaderId, GpuBuffer) {
        let shader = world
            .add_shader(MaterialShader::new(FROM_BUFFER).buffer("colors", "array<vec4<f32>>"))
            .unwrap();
        let buffer = world
            .add_buffer(BufferDesc::from_slice(&[[1f32, 0., 0., 1.]]))
            .unwrap();
        world.set_param(shader, "colors", &buffer).unwrap();
        (shader, buffer)
    }

    #[test]
    fn a_buffer_parameter_colours_its_mesh_as_its_contents_change() {
        let (world, mesh) = cube(unlit(0xffffff));
        let (shader, buffer) = coloured(&world);
        world.set_shader(mesh, Some(shader));
        let Some(mut window) = Window::new(&world) else {
            return;
        };
        window.draw();
        assert!(
            near(window.pixel(50, 50), [255, 0, 0, 255]),
            "{:?}",
            window.pixel(50, 50)
        );
        buffer.write(0, &[[0f32, 0., 1., 1.]]).unwrap();
        window.draw();
        assert!(
            near(window.pixel(50, 50), [0, 0, 255, 255]),
            "{:?}",
            window.pixel(50, 50)
        );
        // A buffer the shader can't bind (a texture's format of ints) is an
        // error, not a frame's.
        let ints = world
            .add_texture(TextureDesc::storage(4, 4, Format::R32Uint))
            .unwrap();
        let reads = world
            .add_shader(
                MaterialShader::new(
                    "#zgx material 1
                     fn surface(s: SurfaceIn) -> Surface {
                         var m = surface_default(s);
                         m.base_color = textureLoad(params.field, vec2(0, 0), 0).rgb;
                         return m;
                     }",
                )
                .param("field", Param::GpuTexture),
            )
            .unwrap();
        assert!(world.set_param(reads, "field", &ints).is_err());
        // A float texture of 32 bits a channel (unfilterable) binds.
        let field = world
            .add_texture(TextureDesc::storage(4, 4, Format::Rgba32Float))
            .unwrap();
        world.set_param(reads, "field", &field).unwrap();
        world.set_shader(mesh, Some(reads));
        window.draw();
        assert_eq!(world.shader_error(reads), None);
    }

    #[test]
    fn another_buffer_draws_as_a_fresh_world_with_it_does() {
        retained_matches_fresh(
            "a buffer swapped",
            |world| {
                let (shader, _) = coloured(world);
                let cube = world.add(
                    Object::mesh(Geometry::cuboid(1., 1., 1.), Material::rgb(0xffffff))
                        .at([0., 0.5, 0.]),
                );
                world.set_shader(cube, Some(shader));
                (vec![shader], cube)
            },
            |world, ids, _| {
                let other = world
                    .add_buffer(BufferDesc::from_slice(&[[0.2f32, 0.9, 0.3, 1.]]))
                    .unwrap();
                world.set_param(ids[0], "colors", &other).unwrap();
            },
        );
    }

    /// A cube moved sideways by `params.lift[0].x`, a buffer's, casting a
    /// shadow: the buffer, its first number `lift`.
    fn lifted_by_data(world: &World, lift: f32) -> (ShaderId, GpuBuffer, ObjectId) {
        let shader = world
            .add_shader(
                MaterialShader::new(
                    "#zgx material 1
                     fn displace(v: VertexIn) -> Displaced {
                         var d: Displaced;
                         d.position = v.position + vec3(params.lift[0].x, 0.0, 0.0);
                         d.normal = v.normal;
                         return d;
                     }",
                )
                .buffer("lift", "array<vec4<f32>>")
                .displaces_by(2.),
            )
            .unwrap();
        let buffer = world
            .add_buffer(BufferDesc::from_slice(&[[lift, 0., 0., 0.]]))
            .unwrap();
        world.set_param(shader, "lift", &buffer).unwrap();
        let cube = world.add(
            Object::mesh(Geometry::cuboid(1., 1., 1.), Material::rgb(0xffffff)).at([0., 0.5, 0.]),
        );
        world.set_shader(cube, Some(shader));
        (shader, buffer, cube)
    }

    #[test]
    fn a_shadow_cast_by_gpu_data_follows_the_apps_writes_to_it() {
        // GB1: a shadow cast by a material's GPU data was drawn once and
        // kept, however the data changed.
        let buffer = std::cell::RefCell::new(None);
        retained_matches_fresh(
            "a buffer written",
            |world| {
                let (shader, made, cube) = lifted_by_data(world, 0.);
                buffer.replace(Some(made));
                (vec![shader], cube)
            },
            |_, _, _| {
                let buffer = buffer.borrow();
                buffer
                    .as_ref()
                    .unwrap()
                    .write(0, &[[1.2f32, 0., 0., 0.]])
                    .unwrap();
            },
        );
    }

    #[test]
    fn a_shadow_cast_by_gpu_data_follows_its_data_swapped() {
        // GB2: two buffers never written (their counts alike), swapped by
        // `set_param`: the shadow is cast by the one bound now. (Through the
        // API this can't fail for GB2's cause alone: binding anew makes a
        // new binding key, which casts anew too. The key itself is
        // `world::materials::tests::data_bound_in_turn_casts_by_which_it_is`.)
        let other = std::cell::RefCell::new(None);
        let setup = |world: &World| {
            let (shader, _, cube) = lifted_by_data(world, 0.);
            let lifted = world
                .add_buffer(BufferDesc::from_slice(&[[1.2f32, 0., 0., 0.]]))
                .unwrap();
            other.replace(Some(lifted));
            (vec![shader], cube)
        };
        retained_matches_fresh("data swapped", setup, |world, ids, _| {
            let other = other.borrow();
            world
                .set_param(ids[0], "lift", other.as_ref().unwrap())
                .unwrap();
        });
        // And back: the first again, as a world that never swapped draws.
        let first = std::cell::RefCell::new(None);
        retained_matches_fresh(
            "data swapped back",
            |world| {
                let (shader, buffer, cube) = lifted_by_data(world, 0.);
                first.replace(Some(buffer));
                (vec![shader], cube)
            },
            |world, ids, _| {
                let lifted = world
                    .add_buffer(BufferDesc::from_slice(&[[1.2f32, 0., 0., 0.]]))
                    .unwrap();
                world.set_param(ids[0], "lift", &lifted).unwrap();
                world
                    .set_param(ids[0], "lift", first.borrow().as_ref().unwrap())
                    .unwrap();
            },
        );
    }

    #[cfg(feature = "lighting")]
    #[test]
    fn a_cascaded_shadow_cast_by_gpu_data_follows_the_apps_writes_to_it() {
        // (This one can't fail for GB1's cause: cascades follow the camera,
        // and `retained_matches_fresh` turns it, which casts them anew
        // whatever the data. It holds cascades to the same result.)
        let buffer = std::cell::RefCell::new(None);
        retained_matches_fresh(
            "a buffer written, cascades",
            |world| {
                world.set_cascades(Some(Cascades::new(2)));
                let (shader, made, cube) = lifted_by_data(world, 0.);
                buffer.replace(Some(made));
                (vec![shader], cube)
            },
            |_, _, _| {
                let buffer = buffer.borrow();
                buffer
                    .as_ref()
                    .unwrap()
                    .write(0, &[[1.2f32, 0., 0., 0.]])
                    .unwrap();
            },
        );
    }

    #[test]
    fn a_shadow_cast_by_gpu_data_follows_a_pass_writing_it() {
        // As the harbor's water is: a compute pass writes what `displace`
        // reads. Against a fresh world whose buffer holds what it wrote.
        let fresh = World::new();
        shadowed(&fresh);
        lifted_by_data(&fresh, 1.2);
        let Some(mut window) = Window::new(&fresh) else {
            return;
        };
        window.draw();
        let expected = window.gpu.readback().unwrap();

        let world = World::new();
        shadowed(&world);
        let (_, buffer, _) = lifted_by_data(&world, 0.);
        let mut window = Window::new(&world).unwrap();
        window.draw();
        let pass = world
            .add_compute(
                Compute::new(
                    "@group(0) @binding(0) var<storage, read_write> lift: array<vec4<f32>>;
                     @compute @workgroup_size(1)
                     fn write_lift() { lift[0] = vec4(1.2, 0.0, 0.0, 0.0); }",
                )
                .buffer("lift", &buffer, Access::ReadWrite)
                .threads([1, 1, 1]),
            )
            .unwrap();
        // Frames until it has compiled and run (once: on demand).
        let start = std::time::Instant::now();
        while buffer.writes() == 0 && start.elapsed() < std::time::Duration::from_secs(10) {
            world.view().invalidate();
            window.draw();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(world.compute_status(pass), Some(ComputeStatus::Ready));
        window.draw();
        let actual = window.gpu.readback().unwrap();
        let differ = actual
            .iter()
            .zip(&expected)
            .filter(|(a, b)| a.abs_diff(**b) > 1)
            .count();
        assert_eq!(differ, 0, "{differ} channels differ from a fresh world's");
    }

    #[test]
    fn a_buffer_parameter_follows_its_buffer_to_a_new_device() {
        // One painter, its device changed (another window's renderer): the
        // buffer's copy is made again there, a new generation, and the
        // binding with it (the old device's copy would be a validation
        // error). The app writes its buffer each frame, as a compute pass's
        // output is: what a device change drops comes again.
        let (world, mesh) = cube(unlit(0xffffff));
        let (shader, buffer) = coloured(&world);
        world.set_shader(mesh, Some(shader));
        let Some(mut first) = Window::new(&world) else {
            return;
        };
        first.draw();
        assert!(near(first.pixel(50, 50), [255, 0, 0, 255]));
        drop(first);
        let mut second = Window::new(&world).unwrap();
        buffer.write(0, &[[0f32, 1., 0., 1.]]).unwrap();
        second.draw();
        second.draw();
        assert!(
            near(second.pixel(50, 50), [0, 255, 0, 255]),
            "{:?}",
            second.pixel(50, 50)
        );
    }
}

/// Each mesh its four numbers (`set_instance_floats`), read as a colour.
const BY_INSTANCE: &str = "#zgx material 1
fn surface(s: SurfaceIn) -> Surface {
    var m = surface_default(s);
    m.base_color = s.instance.rgb;
    return m;
}";

#[test]
fn meshes_of_one_shader_tell_themselves_apart_by_their_numbers() {
    let world = World::new();
    world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
    let shader = world.add_shader(MaterialShader::new(BY_INSTANCE)).unwrap();
    let cubes: Vec<ObjectId> = [-1.2f32, 0., 1.2]
        .into_iter()
        .map(|x| {
            let cube = world.add(
                Object::mesh(Geometry::cuboid(0.8, 0.8, 0.8), unlit(0xffffff)).at([x, 0., 0.]),
            );
            world.set_shader(cube, Some(shader));
            cube
        })
        .collect();
    world.set_instance_floats(cubes[0], Some([1., 0., 0., 0.]));
    world.set_instance_floats(cubes[2], Some([0., 0., 1., 0.]));
    assert_eq!(world.instance_floats(cubes[0]), Some([1., 0., 0., 0.]));
    assert_eq!(world.instance_floats(cubes[1]), None);
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    assert!(
        near(window.pixel(20, 50), [255, 0, 0, 255]),
        "{:?}",
        window.pixel(20, 50)
    );
    assert!(
        near(window.pixel(50, 50), [0, 0, 0, 255]),
        "none: zeros {:?}",
        window.pixel(50, 50)
    );
    assert!(
        near(window.pixel(80, 50), [0, 0, 255, 255]),
        "{:?}",
        window.pixel(80, 50)
    );
    // Changed, the frame draws it so.
    world.set_instance_floats(cubes[1], Some([0., 1., 0., 0.]));
    window.draw();
    assert!(
        near(window.pixel(50, 50), [0, 255, 0, 255]),
        "{:?}",
        window.pixel(50, 50)
    );
    // Not a number: zero.
    world.set_instance_floats(cubes[0], Some([f32::NAN, 1., 0., 0.]));
    assert_eq!(world.instance_floats(cubes[0]), Some([0., 1., 0., 0.]));
}

#[test]
fn displace_and_opacity_read_a_meshs_numbers_in_both_passes() {
    // `v.instance` moves it; `s.instance` cuts it out, its shadow too.
    let world = World::new();
    world.set_shadows(true);
    world.add_light(Light::Directional {
        direction: vec3(0.3, -1., 0.2),
        color: [1.; 3],
        intensity: 3.,
    });
    world.add(
        Object::mesh(Geometry::cuboid(8., 0.2, 8.), Material::rgb(0xc0c0c0)).at([0., -0.6, 0.]),
    );
    let cube = world
        .add(Object::mesh(Geometry::cuboid(1., 1., 1.), Material::rgb(0xc0c0c0)).at([0., 0.5, 0.]));
    world.set_camera(Camera::looking_at([0., 6., 3.], [0., 0., 0.]));
    let shader = world
        .add_shader(MaterialShader::new(
            "#zgx material 1
             fn displace(v: VertexIn) -> Displaced {
                 var d: Displaced;
                 d.position = v.position + vec3(v.instance.x, 0.0, 0.0);
                 d.normal = v.normal;
                 return d;
             }
             fn opacity(s: SurfaceIn) -> f32 {
                 return 1.0 - s.instance.y;
             }",
        ))
        .unwrap();
    world.set_shader(cube, Some(shader));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    window.draw();
    let still = dark(&window);
    assert!(still > 20, "its shadow: {still}");
    world.set_instance_floats(cube, Some([50., 0., 0., 0.]));
    window.draw();
    assert!(
        dark(&window) < 5,
        "moved far by its numbers, its shadow too: {}",
        dark(&window)
    );
    world.set_instance_floats(cube, Some([0., 1., 0., 0.]));
    window.draw();
    assert!(
        dark(&window) < 5,
        "cut out by its numbers, its shadow too: {}",
        dark(&window)
    );
    world.set_instance_floats(cube, None);
    window.draw();
    assert_eq!(dark(&window), still, "as it was");
}

#[test]
fn a_meshs_numbers_changed_draw_as_a_fresh_world_with_them_does() {
    retained_matches_fresh(
        "numbers",
        |world| {
            let shader = world.add_shader(MaterialShader::new(BY_INSTANCE)).unwrap();
            let cube = world.add(
                Object::mesh(Geometry::cuboid(1., 1., 1.), Material::rgb(0xffffff))
                    .at([0., 0.5, 0.]),
            );
            world.set_shader(cube, Some(shader));
            world.set_instance_floats(cube, Some([0.9, 0.2, 0.1, 0.]));
            (vec![shader], cube)
        },
        |world, _, cube| world.set_instance_floats(cube, Some([0.1, 0.3, 0.9, 0.])),
    );
}

/// An emissive wall seen at an exposure of a stop, Neutral tone mapping,
/// the post chain on or off; with a pane in front whose surface gives back
/// what is behind where it is light, and magenta where it isn't: the
/// pane's middle pixel.
#[cfg(feature = "post")]
fn through_a_pane(chain: bool, pane: bool) -> Option<[u8; 4]> {
    let world = World::new();
    world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
    world.set_background_linear(Some([0., 0., 0., 1.]));
    let wall = Material {
        color: [0., 0., 0., 1.],
        emissive: [0.6, 0.3, 0.1],
        roughness: 1.,
        ..Default::default()
    };
    world.add(Object::mesh(Geometry::cuboid(3., 3., 0.2), wall).at([0., 0., -1.]));
    world.set_exposure(1.);
    world.set_tone_mapping(ToneMapping::Neutral);
    if chain {
        world.set_post(Post {
            vignette: Some(Vignette { intensity: 0. }),
            ..Default::default()
        });
    }
    if pane {
        let black = Material {
            color: [0., 0., 0., 1.],
            roughness: 1.,
            ..Default::default()
        };
        let pane = world.add(Object::mesh(Geometry::cuboid(1., 1., 0.2), black));
        let see = world
            .add_shader(
                MaterialShader::new(
                    "#zgx material 1
                     fn surface(s: SurfaceIn) -> Surface {
                         var m = surface_default(s);
                         m.base_color = vec3(0.0);
                         m.emissive = vec3(1.0, 0.0, 1.0);
                         if scene_is_light() {
                             m.emissive = scene_color(s.screen_uv, 0.0);
                         }
                         return m;
                     }",
                )
                .reads_scene(),
            )
            .unwrap();
        world.set_shader(pane, Some(see));
    }
    let mut window = Window::new(&world)?;
    window.draw();
    Some(window.pixel(50, 50))
}

#[cfg(feature = "post")]
#[test]
fn what_is_behind_is_light_where_the_post_chain_is_on() {
    // On: the scene's light, before exposure and tone mapping, so a pane
    // giving it back is as if it weren't there. Off: as displayed, and the
    // material's branch for that is taken.
    let Some(wall) = through_a_pane(true, false) else {
        return;
    };
    let seen = through_a_pane(true, true).unwrap();
    assert!(
        wall.iter().zip(seen).all(|(a, b)| a.abs_diff(b) <= 1),
        "through the pane {seen:?}, the wall {wall:?}"
    );
    let off = through_a_pane(false, true).unwrap();
    assert!(
        // (Magenta, desaturated toward white by the tone curve: the wall
        // is orange.)
        off[0].abs_diff(off[2]) <= 2 && off[1] + 100 < off[0],
        "chain off: the branch for what is displayed, {off:?}"
    );
}

#[test]
fn reading_whether_the_scene_is_light_needs_reads_scene() {
    let world = World::new();
    let error = world
        .add_shader(MaterialShader::new(
            "#zgx material 1
             fn surface(s: SurfaceIn) -> Surface {
                 var m = surface_default(s);
                 if scene_is_light() { m.roughness = 0.5; }
                 return m;
             }",
        ))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("`scene_is_light` reads the scene behind"),
        "{error}"
    );
}

/// A pane over the left half of a glowing backdrop, giving back what is
/// behind it by `call` (`scene_light` or `scene_color`): the pane's pixel
/// and the backdrop's beside it, under `tone` and `stops` of exposure.
fn given_back(call: &str, tone: ToneMapping, stops: f32) -> Option<([u8; 4], [u8; 4])> {
    given_back_in(World::new(), call, tone, stops)
}
fn given_back_in(
    world: World,
    call: &str,
    tone: ToneMapping,
    stops: f32,
) -> Option<([u8; 4], [u8; 4])> {
    world.set_tone_mapping(tone);
    world.set_exposure(stops);
    world.set_camera(Camera::looking_at([0., 0., 3.], [0., 0., 0.]));
    // Black and rough, glowing a colour bright enough to roll off.
    let mut glow = Material::rgb(0x000000).with(0., 1.);
    glow.emissive = [0.9, 0.45, 0.15];
    world.add(Object::mesh(Geometry::cuboid(8., 8., 0.1), glow).at([0., 0., -1.]));
    let pane = world.add(
        Object::mesh(
            Geometry::cuboid(2., 4., 0.05),
            Material::rgb(0x000000).with(0., 1.),
        )
        .at([-1., 0., 0.5]),
    );
    let shader = world
        .add_shader(
            MaterialShader::new(format!(
                "#zgx material 1
                 fn surface(s: SurfaceIn) -> Surface {{
                     var m = surface_default(s);
                     m.emissive = {call}(s.screen_uv, 0.0);
                     return m;
                 }}"
            ))
            .reads_scene(),
        )
        .unwrap();
    world.set_shader(pane, Some(shader));
    let mut window = Window::new(&world)?;
    window.draw();
    Some((window.pixel(30, 50), window.pixel(80, 50)))
}

#[test]
fn scene_light_gives_back_the_light_behind_under_each_tone_map() {
    // The post chain off: `scene_color` is the scene as displayed, and a
    // pane giving it back is mapped twice (brighter, flatter);
    // `scene_light` takes the mapping back, so the pane shows what is
    // behind it as the backdrop beside it does.
    for tone in [ToneMapping::Neutral, ToneMapping::Aces, ToneMapping::Linear] {
        for stops in [0., 1.] {
            let Some((pane, beside)) = given_back("scene_light", tone, stops) else {
                return;
            };
            let off = pane[..3]
                .iter()
                .zip(&beside[..3])
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            eprintln!("{tone:?} {stops}: light {pane:?} beside {beside:?}");
            assert!(
                off <= 3,
                "{tone:?}, {stops} stops: {pane:?} beside {beside:?}"
            );
            if tone != ToneMapping::Linear {
                let (pane, beside) = given_back("scene_color", tone, stops).unwrap();
                eprintln!("{tone:?} {stops}: color {pane:?} beside {beside:?}");
                let off = pane[..3]
                    .iter()
                    .zip(&beside[..3])
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap();
                assert!(
                    off > 6,
                    "the control, mapped twice: {pane:?} beside {beside:?}"
                );
            }
        }
    }
}

#[cfg(feature = "post")]
#[test]
fn scene_light_is_scene_color_with_the_post_chain_on() {
    // The chain on: what is behind is light already, and given back it is
    // exposed and mapped once, with the rest.
    for tone in [ToneMapping::Neutral, ToneMapping::Aces] {
        for call in ["scene_light", "scene_color"] {
            let world = World::new();
            world.set_post(Post {
                bloom: Some(Bloom { intensity: 0. }),
                ..Default::default()
            });
            let Some((pane, beside)) = given_back_in(world, call, tone, 0.) else {
                return;
            };
            let off = pane[..3]
                .iter()
                .zip(&beside[..3])
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(off <= 3, "{tone:?} {call}: {pane:?} beside {beside:?}");
        }
    }
}

#[test]
fn a_material_reads_six_mipmapped_textures_and_no_more() {
    let textures = |n: usize| {
        (0..n).fold(
            MaterialShader::new(
                "#zgx material 1
                 fn surface(s: SurfaceIn) -> Surface {
                     var m = surface_default(s);
                     let others = textureSample(params.t1, material_sampler, s.uv)
                         + textureSample(params.t2, material_sampler, s.uv)
                         + textureSample(params.t3, material_sampler, s.uv)
                         + textureSample(params.t4, material_sampler, s.uv)
                         + textureSample(params.t5, material_sampler, s.uv);
                     // Level 3 of an 8 by 8 checker: its one texel, grey.
                     m.base_color = textureSampleLevel(params.t0, material_sampler, s.uv, 3.0).rgb
                         + others.rgb * 0.0;
                     return m;
                 }",
            ),
            |shader, i| shader.param(&format!("t{i}"), Param::Texture),
        )
    };
    let world = World::new();
    let e = world.add_shader(textures(7)).unwrap_err();
    assert!(e.message.contains("6 at most"), "{e}");
    // Six, in a world binding much else: an environment, the sun's shadow
    // (and cascades and clustered lights), a mesh with a map of its own.
    world.set_environment(Some(Environment::studio()));
    world.set_shadows(true);
    world.add_light(Light::Directional {
        direction: vec3(0.2, -1., -0.3),
        color: [1.; 3],
        intensity: 1.,
    });
    #[cfg(feature = "lighting")]
    {
        world.set_cascades(Some(Cascades::new(2)));
        for i in 0..4 {
            world.add_light(Light::Point {
                position: vec3(i as f32 - 1.5, 1., 1.),
                color: [1.; 3],
                intensity: 0.1,
                range: 3.,
            });
        }
    }
    let mut plain = Material::rgb(0xffffff);
    plain.unlit = true;
    let square = world.add(Object::mesh(Geometry::cuboid(2., 2., 0.1), plain));
    world.set_texture(
        square,
        Some(Texture::from_rgba8(1, 1, vec![255; 4]).unwrap()),
    );
    world.set_camera(Camera::looking_at([0., 0., 3.], [0., 0., 0.]));
    let shader = world.add_shader(textures(6)).unwrap();
    let checker: Vec<u8> = (0..64)
        .flat_map(|i| {
            let on = (i % 8 + i / 8) % 2 == 0;
            [
                if on { 255 } else { 0 },
                if on { 255 } else { 0 },
                if on { 255 } else { 0 },
                255,
            ]
        })
        .collect();
    world
        .set_param(shader, "t0", Texture::from_rgba8(8, 8, checker).unwrap())
        .unwrap();
    let white = Texture::from_rgba8(1, 1, vec![255; 4]).unwrap();
    for i in 1..6 {
        world.set_param(shader, &format!("t{i}"), &white).unwrap();
    }
    world.set_shader(square, Some(shader));
    let Some(mut window) = Window::new(&world) else {
        return;
    };
    // Its pipelines compile off the frame; till then it isn't drawn here.
    let start = std::time::Instant::now();
    window.draw();
    while world.compiling() && start.elapsed() < std::time::Duration::from_secs(20) {
        std::thread::sleep(std::time::Duration::from_millis(5));
        window.draw();
    }
    window.draw();
    assert_eq!(world.shader_error(shader), None);
    // Its level 3: one texel, grey, the same across the square where the
    // largest level would be a checker. (128: today's levels average the
    // sRGB bytes, not the light; in linear light it would be 188.)
    let row: Vec<[u8; 4]> = (36..64).step_by(4).map(|x| window.pixel(x, 50)).collect();
    let grey = row[0];
    assert!(
        row.iter().all(|p| p == &grey) && (110..200).contains(&grey[0]) && grey[0] == grey[2],
        "{row:?}"
    );
}
