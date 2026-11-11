//! Retained 3D scenes for zgx.
//!
//! A [`World`] holds meshes ([`Object`]: a [`Geometry`] in a [`Material`],
//! placed by a [`Transform`], grouped under parents), [`Light`]s and a
//! [`Camera`]. Its GPU view ([`World::view`]) shows it in any box
//! (`zgx::compose::gpu_view`), drawn with depth, 4x multisampling and
//! metallic-roughness shading. It draws a frame only when the world changes:
//! a still scene costs nothing, and its pipelines and buffers are made with
//! its first frame.
//!
//! With the `gltf` feature, `Model` reads glTF 2.0 files (`.glb`, or
//! `.gltf` and the buffers beside it) and `World::add_model` adds one as a
//! group to place, turn and scale as any object.
//!
//! ```no_run
//! use zgx::compose::prelude::*;
//! use zgx_3d::*;
//!
//! let world = World::new();
//! let cube = world.add(Object::mesh(Geometry::cuboid(1., 1., 1.), Material::rgb(0x4a90e2)));
//! world.add_light(Light::Directional {
//!     direction: vec3(-1., -2., -1.),
//!     color: [1., 1., 1.],
//!     intensity: 3.,
//! });
//! world.set_camera(Camera::looking_at([2., 1.5, 3.], [0., 0., 0.]));
//! let view = gpu_view("A cube", world.view()).w_full().h(320.);
//! // Turning it draws one frame:
//! world.update(cube, |o| o.transform.rotation = Quat::from_euler(0., 0.5, 0.));
//! ```
#![warn(missing_docs)]

#[cfg(all(doctest, feature = "gltf"))]
#[doc = include_str!("../../../docs/3d.md")]
mod guide {}

#[cfg(feature = "gltf")]
mod animation;
#[cfg(feature = "gltf")]
mod animator;
mod assets;
#[cfg(feature = "compute")]
mod buffer;
mod caps;
#[cfg(feature = "compute")]
mod compute;
mod environment;
mod geometry;
#[cfg(feature = "gltf")]
mod gltf;
#[cfg(feature = "gltf")]
pub mod ik;
#[cfg(feature = "shaders")]
mod material;
mod math;
mod memory;
mod orbit;
mod painter;
#[cfg(feature = "particles")]
mod particles;
mod pick;
mod render;
#[cfg(feature = "compute")]
mod shader_sandbox;
mod texture;
mod world;

#[cfg(feature = "gltf")]
pub use animation::{Clip, Placed};
#[cfg(feature = "gltf")]
pub use animator::{
    AnimatorError, AnimatorId, AnimatorUpdate, Fired, Layer, MAX_LAYERS, Mask, Play, RootMotion,
};
#[cfg(feature = "compute")]
pub use buffer::{
    BufferDesc, DEFAULT_MEMORY_CAP, DEFAULT_READBACK_BUDGET, Format, GpuBuffer, GpuDataError,
    GpuTexture, ReadError, Readback, TextureDesc, WriteError,
};
pub use caps::{Backend, Caps, Compressed, Tier};
#[cfg(feature = "compute")]
pub use compute::{Access, Compute, ComputeBudget, ComputeError, ComputeId, ComputeStatus, Point};
#[cfg(feature = "compute")]
#[doc(hidden)]
pub use compute::{compile_threads, stall_sandboxed_compiles};
pub use environment::Environment;
pub use geometry::{Geometry, MorphTarget, Vertex};
#[cfg(feature = "gltf")]
pub use gltf::{Model, ModelError};
#[cfg(feature = "shaders")]
pub use material::{MATERIAL_VERSION, MaterialShader, Param, ParamValue, ShaderError, ShaderId};
pub use math::{Mat4, Quat, Transform, Vec3, vec3};
pub use memory::Memory;
pub use orbit::{Answer, Orbit, OrbitControls};
#[cfg(feature = "particles")]
pub use particles::{
    Blend, CPU_PARTICLES, CPU_WORLD_PARTICLES, Collide, Emitter, EmitterError, EmitterId,
    EmitterStatus, Force, Response, Shape, Spawn,
};
pub use pick::Hit;
pub use texture::{Map, MapPlacement, Texture, Wrap};
#[cfg(feature = "gi")]
pub use world::gi::{MAX_PROBE_VOLUMES, MAX_PROBES, ProbeVolume, ProbesId};
#[cfg(feature = "lighting")]
pub use world::lighting::{Cascades, PointLight};
pub use world::{
    Camera, Clearcoat, Light, LightId, MAX_JOINTS, Material, Object, ObjectId, POINT_LIGHTS, Skin,
    Specular, Spot, ToneMapping, Transmission, World, srgb_to_linear,
};
