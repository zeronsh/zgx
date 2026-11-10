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
        Some(Self { ui, gpu })
    }
    fn draw(&mut self) -> zgx_gpu::GpuStats {
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
fn retained_matches_fresh(
    name: &str,
    setup: impl Fn(&World) -> (Vec<ShaderId>, ObjectId),
    after: impl Fn(&World, &[ShaderId], ObjectId),
) {
    let lit = |world: &World| {
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
    };
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
