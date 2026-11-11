//! The rests-once-off row for a custom material's compile
//! (`Reason::ShaderCompiling`), in a test binary of its own: its compile is
//! held on its worker (`ZGX_TEST_COMPILE_DELAY_MS`, read process-wide) so
//! the hold is there to remove however fast the machine.
use zgx_3d::*;

#[path = "common/settle.rs"]
mod harness;
use harness::{caught, rests_caught, shows};

/// The compile worker held 400 ms a pipeline, set once for the binary
/// before either test's first frame (no thread reads the environment
/// before then).
fn hold_worker() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    // SAFETY: set under `ONCE`, before any frame of this binary starts
    // the worker that reads it.
    ONCE.call_once(|| unsafe { std::env::set_var("ZGX_TEST_COMPILE_DELAY_MS", "400") });
}

#[test]
fn a_mesh_shader_and_a_surface_layer_taken_off_while_they_compile() {
    hold_worker();
    rests_caught("shader compiling", |world, mesh, window| {
        // A mesh's own shader with a layer over it, as the shader lane's
        // compile test has them: compiled on the (held) worker. (A layer
        // alone over stock meshes isn't: see the next row.)
        let red = world
            .add_shader(MaterialShader::new(
                "#zgx material 1
                 fn surface(s: SurfaceIn) -> Surface {
                     var m = surface_default(s);
                     m.base_color = vec3(1.0, 0.0, 0.0);
                     return m;
                 }",
            ))
            .unwrap();
        world.set_shader(mesh, Some(red));
        let layer = world
            .add_surface_layer(MaterialShader::new(
                "#zgx material 1
                 fn surface(s: SurfaceIn) -> Surface {
                     var m = surface_default(s);
                     m.base_color *= 0.5;
                     return m;
                 }",
            ))
            .unwrap();
        if !caught(world, window) {
            return None;
        }
        Some(Box::new(move |world| {
            world.remove_surface_layer(layer);
            world.set_shader(mesh, None);
        }))
    });
}

#[test]
fn a_surface_layer_alone_draws_in_its_first_frame_and_rests() {
    // A layer over stock meshes holds no reason because nothing of it is
    // left to wait for: the stock pipelines are made with it on the frame
    // that sees it (`custom::relayer`), not on the compile worker, which is
    // held here. Only a mesh shader's pipelines (each with every layer)
    // compile there, holding `ShaderCompiling` for it and its layers.
    hold_worker();
    let (world, _) = harness::world();
    let Some(mut window) = harness::Window::new(&world) else {
        return;
    };
    window.draw(&world);
    let layer = world
        .add_surface_layer(MaterialShader::new(
            "#zgx material 1
             fn surface(s: SurfaceIn) -> Surface {
                 var m = surface_default(s);
                 m.base_color *= 0.5;
                 return m;
             }",
        ))
        .unwrap();
    window.draw(&world);
    assert!(
        world.shader_ready(layer),
        "drawn with it from its first frame"
    );
    assert!(!world.compiling(), "nothing of it on the worker");
    assert!(!world.view().animating(), "nothing holds the world drawing");
}

#[test]
fn a_mesh_shader_and_a_surface_layer_show_once_compiled() {
    // Compiled on the held worker, drawn as a window draws: the world
    // rests only with its picture drawn by both.
    hold_worker();
    shows("shader compiled", |world, mesh, _| {
        let red = world
            .add_shader(MaterialShader::new(
                "#zgx material 1
                 fn surface(s: SurfaceIn) -> Surface {
                     var m = surface_default(s);
                     m.base_color = vec3(1.0, 0.0, 0.0);
                     return m;
                 }",
            ))
            .unwrap();
        world.set_shader(mesh, Some(red));
        let layer = world
            .add_surface_layer(MaterialShader::new(
                "#zgx material 1
                 fn surface(s: SurfaceIn) -> Surface {
                     var m = surface_default(s);
                     m.base_color *= 0.5;
                     return m;
                 }",
            ))
            .unwrap();
        Some(Box::new(move |world| {
            world.shader_ready(red) && world.shader_ready(layer) && !world.compiling()
        }))
    });
}
