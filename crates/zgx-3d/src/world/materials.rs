//! A world's custom materials (`World::add_shader`, `set_shader`,
//! `set_param`) and surface layers (`World::add_surface_layer`).
use super::{Inner, ObjectId, World};
use crate::material::{
    LAYER_BYTES, MAX_LAYERS, MAX_SHADERS, MaterialShader, ParamValue, Shader, ShaderError,
    ShaderId, layer_offsets,
};
use std::rc::Rc;

/// A mesh's custom material: one shader, compared by which.
#[derive(Clone, Debug)]
pub(crate) struct ShaderRef(pub(crate) Rc<Shader>);
impl PartialEq for ShaderRef {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Inner {
    /// Shader `id`, if this world has it.
    fn shader_of(&self, id: ShaderId) -> Option<&Rc<Shader>> {
        self.shaders.list.get((id.0 as usize).checked_sub(1)?)
    }
}

impl World {
    /// Add `shader`, a custom material for meshes (`set_shader`): its text
    /// read and compiled now, not when a frame first draws it, its errors
    /// at its own lines.
    pub fn add_shader(&self, shader: MaterialShader) -> Result<ShaderId, ShaderError> {
        self.add_checked(shader, false)
    }
    /// Draw mesh `id` with `shader` (`None`: its material alone). Its
    /// `Material` and maps are what `surface_default` gives the shader.
    pub fn set_shader(&self, id: ObjectId, shader: Option<ShaderId>) {
        self.change(|inner| {
            if inner.live(id.index, id.generation).is_none() {
                return false;
            }
            let shader = shader
                .and_then(|s| inner.shader_of(s))
                .filter(|s| !inner.shaders.layers.iter().any(|l| Rc::ptr_eq(l, s)))
                .cloned();
            let extras = inner.extras_mut(id.index, id.generation);
            let changed = extras.shader.as_ref().map(|s| s.0.id) != shader.as_ref().map(|s| s.id);
            extras.shader = shader.map(ShaderRef);
            inner.tidy_extras(id.index);
            changed
        });
    }
    /// Mesh `id`'s custom material, if it has one.
    pub fn shader(&self, id: ObjectId) -> Option<ShaderId> {
        let inner = self.inner.borrow();
        let extras = inner.extras(id.index as usize, id.generation)?;
        extras.shader.as_ref().map(|s| ShaderId(s.0.id))
    }
    /// Set `shader`'s parameter `name` (`MaterialShader::param`) to
    /// `value`: its numbers are written before the next frame, a texture
    /// bound with it. An error if it has no such parameter or it holds
    /// another kind.
    pub fn set_param(
        &self,
        shader: ShaderId,
        name: &str,
        value: impl Into<ParamValue>,
    ) -> Result<(), ShaderError> {
        let value = value.into();
        let texture = matches!(value, ParamValue::Texture(_));
        let found = self.inner.borrow().shader_of(shader).cloned();
        let message = |message| ShaderError {
            line: 0,
            column: 0,
            message,
        };
        let shader = found.ok_or_else(|| message(format!("no shader {}", shader.0)))?;
        if shader.set(name, value).map_err(message)? {
            if texture || shader.displace || shader.opacity {
                // Its meshes' bindings hold its textures: made again. And
                // where it moves or cuts out its meshes, their shadows
                // change: a frame keeping the last's casters would keep
                // their old shadow (`Kept`).
                self.change(|_| true);
            } else {
                // Its numbers alone: written before the next frame, kept
                // or not.
                self.change_view(|_| true);
            }
        }
        Ok(())
    }
    /// Add `shader` as a surface layer: its `surface` runs after every lit
    /// mesh's own, `surface_default` giving what that made (as rain makes
    /// everything wet). Layers run in the order added. Its parameters are
    /// numbers, set by `set_param`.
    pub fn add_surface_layer(&self, shader: MaterialShader) -> Result<ShaderId, ShaderError> {
        self.add_checked(shader, true)
    }
    /// What the device refused of `shader` when a frame compiled or bound it
    /// (a limit its textures and a mesh's own maps pass together, say), if
    /// anything: its meshes then draw without it. Its text was checked when
    /// added; this is what only a device can say.
    pub fn shader_error(&self, shader: ShaderId) -> Option<String> {
        let inner = self.inner.borrow();
        inner.shader_of(shader)?.failed.borrow().clone()
    }
    /// Whether `shader`'s pipelines are compiled and none is still
    /// compiling: its meshes draw by it from the next frame. A shader's
    /// pipelines compile off the frame once a mesh using it is in a frame
    /// (in view or not: culling doesn't wait for them), and until then a
    /// mesh whose frame binds otherwise than the plain pipeline's (lit by an
    /// environment, clusters, cascades or probes) isn't drawn. Show such a
    /// mesh once this says so, after a frame with it out of view, to avoid
    /// its popping in.
    pub fn shader_ready(&self, shader: ShaderId) -> bool {
        let inner = self.inner.borrow();
        inner
            .shader_of(shader)
            .is_some_and(|s| s.compiled.get() && s.compiling.get() == 0)
    }
    /// Whether a custom material's pipeline is compiling (off the frame:
    /// its meshes draw by the plain pipeline, or not at all, till it's
    /// ready). For tests to wait on.
    #[doc(hidden)]
    pub fn compiling(&self) -> bool {
        self.inner.borrow().settle.holds(crate::render::COMPILING)
    }
    /// Remove surface layer `layer`.
    pub fn remove_surface_layer(&self, layer: ShaderId) {
        self.change(|inner| {
            let before = inner.shaders.layers.len();
            inner.shaders.layers.retain(|l| l.id != layer.0);
            inner.shaders.layers.len() != before
        });
    }
    fn add_checked(&self, shader: MaterialShader, layer: bool) -> Result<ShaderId, ShaderError> {
        let refuse = |message: String| ShaderError {
            line: 0,
            column: 0,
            message,
        };
        {
            let shaders = &self.inner.borrow().shaders;
            if shaders.list.len() >= MAX_SHADERS {
                return Err(refuse(format!(
                    "{MAX_SHADERS} shaders: a world holds no more"
                )));
            }
            if layer && shaders.layers.len() >= MAX_LAYERS {
                return Err(refuse(format!(
                    "{MAX_LAYERS} surface layers: a world runs no more"
                )));
            }
        }
        let id = self.inner.borrow().shaders.list.len() as u32 + 1;
        let made = Rc::new(Shader::new(shader, id, layer)?);
        if layer {
            let inner = self.inner.borrow();
            let layers = inner.shaders.layers.iter().map(|l| &**l);
            let (_, size) = layer_offsets(layers.chain([&*made]));
            if size > LAYER_BYTES {
                return Err(ShaderError {
                    line: 0,
                    column: 0,
                    message: format!(
                        "the surface layers' parameters take {size} bytes: {LAYER_BYTES} at most"
                    ),
                });
            }
        }
        self.change(|inner| {
            inner.shaders.list.push(made.clone());
            if layer {
                inner.shaders.layers.push(made);
            }
            layer
        });
        Ok(ShaderId(id))
    }
}
