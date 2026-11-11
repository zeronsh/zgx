//! The rests-once-off row for a custom material's compile
//! (`Reason::ShaderCompiling`), in a test binary of its own: its compile is
//! held on its worker (`ZGX_TEST_COMPILE_DELAY_MS`, read process-wide) so
//! the hold is there to remove however fast the machine.
use zgx_3d::*;

#[path = "common/settle.rs"]
mod harness;
use harness::{caught, rests_caught};

#[test]
fn a_mesh_shader_and_a_surface_layer_taken_off_while_they_compile() {
    // The only test here: nothing else reads the variable meanwhile.
    // SAFETY: no other thread of this binary reads the environment yet.
    unsafe { std::env::set_var("ZGX_TEST_COMPILE_DELAY_MS", "400") };
    rests_caught("shader compiling", |world, mesh, window| {
        // A mesh's own shader with a layer over it, as the shader lane's
        // compile test has them: compiled on the (held) worker. (A layer
        // alone over stock meshes is never caught compiling here, held or
        // not: the shader lane's to say whether it compiles on the worker.)
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
