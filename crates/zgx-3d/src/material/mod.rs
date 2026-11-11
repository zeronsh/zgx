//! Custom materials: WGSL functions an app plugs into the lit pipeline
//! (`World::add_shader`), checked when added, drawn by the meshes given one
//! (`World::set_shader`) or run over every lit mesh as a surface layer
//! (`World::add_surface_layer`).
//!
//! A shader's text is the app's: its top-level names are its own (renamed
//! where it's put beside the engine's and other shaders'), its `params` the
//! values `World::set_param` gives. Nothing of it runs for a mesh without a
//! shader, in a world without layers.
use std::{cell::RefCell, fmt};
use zgx_gpu::wgpu::naga;

/// The version of the inputs a shader is written against: its first line
/// says `#zgx material 1`.
pub const MATERIAL_VERSION: u32 = 1;

/// What a [`MaterialShader`]'s parameter holds: `params.<name>` in its WGSL
/// is an `f32`, a `vec2<f32>`, `vec3<f32>` or `vec4<f32>` (a colour is a
/// linear RGBA `vec4<f32>`), or a `texture_2d<f32>` to sample with
/// `material_sampler`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Param {
    /// An `f32`.
    Float,
    /// A `vec2<f32>`.
    Vec2,
    /// A `vec3<f32>`.
    Vec3,
    /// A `vec4<f32>`.
    Vec4,
    /// A linear RGBA colour, a `vec4<f32>`.
    Color,
    /// A `texture_2d<f32>` of colours, read in linear light (decoded from
    /// sRGB), white until set. A mesh's shader only: a surface layer's
    /// parameters are numbers.
    Texture,
    /// A `texture_2d<f32>` of data (heights, normals, masks), read as
    /// stored, white until set. A mesh's shader only.
    DataTexture,
}
impl Param {
    /// Its WGSL type, its alignment and its size in a uniform (none for a
    /// texture).
    fn layout(self) -> Option<(&'static str, u32, u32)> {
        match self {
            Param::Float => Some(("f32", 4, 4)),
            Param::Vec2 => Some(("vec2<f32>", 8, 8)),
            Param::Vec3 => Some(("vec3<f32>", 16, 12)),
            Param::Vec4 | Param::Color => Some(("vec4<f32>", 16, 16)),
            Param::Texture | Param::DataTexture => None,
        }
    }
}

/// A value for a parameter (`World::set_param`): numbers by their count, or
/// a texture.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum ParamValue {
    /// For a `Param::Float`.
    Float(f32),
    /// For a `Param::Vec2`.
    Vec2([f32; 2]),
    /// For a `Param::Vec3`.
    Vec3([f32; 3]),
    /// For a `Param::Vec4` or `Param::Color`.
    Vec4([f32; 4]),
    /// For a `Param::Texture`.
    Texture(crate::Texture),
}
impl From<f32> for ParamValue {
    fn from(v: f32) -> Self {
        Self::Float(v)
    }
}
impl From<[f32; 2]> for ParamValue {
    fn from(v: [f32; 2]) -> Self {
        Self::Vec2(v)
    }
}
impl From<[f32; 3]> for ParamValue {
    fn from(v: [f32; 3]) -> Self {
        Self::Vec3(v)
    }
}
impl From<[f32; 4]> for ParamValue {
    fn from(v: [f32; 4]) -> Self {
        Self::Vec4(v)
    }
}
impl From<crate::Texture> for ParamValue {
    fn from(v: crate::Texture) -> Self {
        Self::Texture(v)
    }
}
impl From<&crate::Texture> for ParamValue {
    fn from(v: &crate::Texture) -> Self {
        Self::Texture(v.clone())
    }
}

/// A custom material: WGSL functions (`displace`, `surface`, `opacity`, any
/// left out) against the inputs of `#zgx material 1`, with the parameters
/// it reads. See docs/3d.md, "Custom materials", for the inputs and what
/// is promised of them.
///
/// ```
/// use zgx_3d::*;
/// let world = World::new();
/// let tint = world
///     .add_shader(
///         MaterialShader::new(
///             "#zgx material 1
///              fn surface(s: SurfaceIn) -> Surface {
///                  var m = surface_default(s);
///                  m.base_color *= params.tint.rgb;
///                  return m;
///              }",
///         )
///         .param("tint", Param::Color),
///     )
///     .unwrap();
/// world.set_param(tint, "tint", [1., 0.5, 0.2, 1.]);
/// ```
#[derive(Clone, Debug)]
pub struct MaterialShader {
    source: String,
    params: Vec<(String, Param)>,
    reads_scene: bool,
    animated: bool,
    reach: Option<f32>,
}
impl MaterialShader {
    /// A shader of `source`: WGSL, its first line `#zgx material 1`.
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            params: Vec::new(),
            reads_scene: false,
            animated: false,
            reach: None,
        }
    }
    /// A parameter it reads as `params.<name>`, set by `World::set_param`
    /// (zero, or a white texture, until then).
    pub fn param(mut self, name: &str, kind: Param) -> Self {
        self.params.push((name.to_owned(), kind));
        self
    }
    /// It reads the scene behind it (`scene_color`): the world draws the
    /// opaque meshes into a picture first, as for glass.
    pub fn reads_scene(mut self) -> Self {
        self.reads_scene = true;
        self
    }
    /// Its `displace` moves a vertex at most `metres` from where its mesh
    /// puts it: the mesh's box grows by that for culling and the sun's
    /// shadow's fit. A shader that displaces without saying so is never
    /// culled, and its displaced parts may fall outside its shadow.
    pub fn displaces_by(mut self, metres: f32) -> Self {
        self.reach = Some(metres);
        self
    }
    /// It reads time (`frame.time`, `frame.delta`, `frame.index`): the world
    /// keeps drawing while a mesh with it is in view. A shader that doesn't
    /// say so can't read them.
    pub fn animated(mut self) -> Self {
        self.animated = true;
        self
    }
}

/// A shader added to a world (`World::add_shader`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShaderId(pub(crate) u32);

/// Why a shader wasn't added: what is wrong and where, at a line and column
/// of the app's own text (1 for its first; 0 where no one line is).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderError {
    /// The line of the shader's text, from 1.
    pub line: u32,
    /// The column, from 1, in bytes.
    pub column: u32,
    /// What is wrong.
    pub message: String,
}
impl fmt::Display for ShaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}
impl std::error::Error for ShaderError {}
fn error(line: u32, column: u32, message: impl Into<String>) -> ShaderError {
    ShaderError {
        line,
        column,
        message: message.into(),
    }
}

/// A parameter's place: a byte offset in its shader's uniform, or its
/// texture's index.
#[derive(Clone, Debug)]
pub(crate) struct Slot {
    pub(crate) name: String,
    pub(crate) kind: Param,
    pub(crate) at: u32,
}
/// Its parameters' values as they are now.
#[derive(Debug, Default)]
pub(crate) struct Values {
    /// Its uniform's bytes.
    pub(crate) bytes: Vec<u8>,
    pub(crate) textures: Vec<Option<crate::Texture>>,
    /// Counts changes to `bytes`, and to `textures`.
    pub(crate) numbers: u64,
    pub(crate) pictures: u64,
}
/// A shader as added: its text read and checked.
#[derive(Debug)]
pub(crate) struct Shader {
    /// Its id's number (`ShaderId`), from 1.
    pub(crate) id: u32,
    /// Its text, the version line blanked (so its lines stay where they
    /// were).
    code: String,
    /// Its top-level names: renamed where it's put beside others.
    names: Vec<String>,
    pub(crate) displace: bool,
    pub(crate) surface: bool,
    pub(crate) opacity: bool,
    pub(crate) slots: Vec<Slot>,
    /// Its uniform's size (0: it has no numbers), and its textures.
    pub(crate) uniform_size: u32,
    pub(crate) textures: u32,
    pub(crate) reads_scene: bool,
    pub(crate) animated: bool,
    /// How far its `displace` moves a vertex, if it says (`displaces_by`).
    pub(crate) reach: Option<f32>,
    pub(crate) values: RefCell<Values>,
    /// What the device refused of it, if anything (`World::shader_error`):
    /// its meshes then draw without it.
    pub(crate) failed: RefCell<Option<String>>,
}

/// Bounds on what an app may hand a world: a shader's text, its parameters
/// and their names, a world's shaders and surface layers. They keep its
/// memory and compile times in reason; they don't make a shader safe to
/// run. Custom materials are trusted code (a `loop {}` holds the device):
/// a plugin or script reaches them only through a sandboxed mode, not yet
/// built.
pub(crate) const MAX_SOURCE: usize = 256 * 1024;
pub(crate) const MAX_PARAMS: usize = 64;
pub(crate) const MAX_NAME: usize = 64;
pub(crate) const MAX_SHADERS: usize = 1024;
pub(crate) const MAX_LAYERS: usize = 8;
/// The farthest `displaces_by` says a vertex moves, in metres.
pub(crate) const MAX_REACH: f32 = 1e4;
/// Textures a mesh's shader may read: with a mesh's own maps, the shadow
/// map, an environment and what is behind, within the 16 a stage may.
pub(crate) const MAX_TEXTURES: u32 = 4;
/// Bytes of the frame for surface layers' parameters (`FRAME_TAIL`).
pub(crate) const LAYER_BYTES: u32 = 1024;
/// Bytes of the frame past the engine's own for custom materials: the view,
/// projection, size and time, then the layers' parameters.
pub(crate) const FRAME_TAIL: u64 = 160 + LAYER_BYTES as u64;
/// Group 1's bindings for a mesh's shader: its sampler, its uniform, its
/// textures from `TEXTURE_BINDING`, and what is behind and its sampler.
pub(crate) const SAMPLER_BINDING: u32 = 15;
pub(crate) const UNIFORM_BINDING: u32 = 16;
pub(crate) const TEXTURE_BINDING: u32 = 17;
pub(crate) const SCENE_BINDING: u32 = 30;

impl Shader {
    /// `shader` read and checked, as a mesh's (`layer` false) or a surface
    /// layer's, numbered `id`.
    pub(crate) fn new(shader: MaterialShader, id: u32, layer: bool) -> Result<Self, ShaderError> {
        if shader.source.len() > MAX_SOURCE {
            return Err(error(
                0,
                0,
                format!(
                    "{} bytes of text: {MAX_SOURCE} at most",
                    shader.source.len()
                ),
            ));
        }
        if shader.params.len() > MAX_PARAMS {
            return Err(error(
                0,
                0,
                format!("{} parameters: {MAX_PARAMS} at most", shader.params.len()),
            ));
        }
        if shader
            .reach
            .is_some_and(|r| !(0.0..=MAX_REACH).contains(&r))
        {
            return Err(error(
                0,
                0,
                format!("`displaces_by` takes 0 to {MAX_REACH} metres"),
            ));
        }
        let code = version(&shader.source)?;
        let names = declared(&code);
        let has = |name: &str| names.iter().any(|(n, f)| *f && n == name);
        let (displace, surface, opacity) = (has("displace"), has("surface"), has("opacity"));
        if layer && (displace || opacity) {
            let name = if displace { "displace" } else { "opacity" };
            return Err(error(
                line_of_fn(&code, name),
                1,
                format!("a surface layer runs `surface` alone: `{name}` is a mesh's shader's"),
            ));
        }
        if layer && !surface {
            return Err(error(0, 0, "a surface layer needs `fn surface`"));
        }
        // What it reads that it must say it does.
        for (at, name, next) in uses(&code) {
            let (line, column) = at;
            match name {
                "scene_color" if !shader.reads_scene || layer => {
                    return Err(error(
                        line,
                        column,
                        if layer {
                            "a surface layer can't read the scene behind".to_owned()
                        } else {
                            "`scene_color` reads the scene behind: say `.reads_scene()`".to_owned()
                        },
                    ));
                }
                "scene_depth" => {
                    return Err(error(
                        line,
                        column,
                        "`scene_depth` comes with the depth prepass: not yet",
                    ));
                }
                "frame" if !shader.animated && matches!(next, Some("time" | "delta" | "index")) => {
                    return Err(error(
                        line,
                        column,
                        format!(
                            "`frame.{}` changes by itself: say `.animated()`",
                            next.unwrap_or_default()
                        ),
                    ));
                }
                _ => {}
            }
        }
        // Its parameters' places, as WGSL lays a uniform out.
        let mut slots = Vec::new();
        let (mut offset, mut align, mut textures) = (0u32, 4u32, 0u32);
        for (name, kind) in &shader.params {
            if !is_identifier(name)
                || name.len() > MAX_NAME
                || slots.iter().any(|s: &Slot| s.name == *name)
            {
                return Err(error(0, 0, format!("parameter `{name}`: a new WGSL name")));
            }
            let at = match kind.layout() {
                Some((_, a, size)) => {
                    let at = offset.next_multiple_of(a);
                    offset = at + size;
                    align = align.max(a);
                    at
                }
                None if layer => {
                    return Err(error(
                        0,
                        0,
                        format!("parameter `{name}`: a surface layer's are numbers, not textures"),
                    ));
                }
                None => {
                    textures += 1;
                    textures - 1
                }
            };
            slots.push(Slot {
                name: name.clone(),
                kind: *kind,
                at,
            });
        }
        if textures > MAX_TEXTURES {
            return Err(error(
                0,
                0,
                format!("{textures} texture parameters: a shader reads {MAX_TEXTURES} at most"),
            ));
        }
        let uniform_size = offset.next_multiple_of(align.max(16)) * u32::from(offset > 0);
        let made = Self {
            id,
            code,
            names: names.into_iter().map(|(n, _)| n).collect(),
            displace,
            surface,
            opacity,
            slots,
            uniform_size,
            textures,
            reads_scene: shader.reads_scene,
            animated: shader.animated,
            reach: shader.reach,
            values: RefCell::new(Values {
                bytes: vec![0; uniform_size as usize],
                textures: vec![None; textures as usize],
                numbers: 0,
                pictures: 0,
            }),
            failed: RefCell::new(None),
        };
        made.check(layer)?;
        Ok(made)
    }

    /// It compiled as a mesh's shader in a textured, multisampled pipeline,
    /// or as a layer in the plain one: its errors at its own lines.
    fn check(&self, layer: bool) -> Result<(), ShaderError> {
        let (mesh, layers) = if layer {
            (None, std::slice::from_ref(self))
        } else {
            (Some(self), &[][..])
        };
        let custom = Custom::new(mesh, layers.iter());
        let engine = crate::render::engine_text(!layer, custom.opacity, custom.animated);
        let text = format!("{engine}{}", custom.code);
        // The assembled text's line (from 1) its own first is at.
        let engine_lines = engine.matches('\n').count() as u32;
        let start = engine_lines + custom.user_line + 1;
        let found = match naga::front::wgsl::parse_str(&text) {
            Err(e) => Some((e.location(&text), e.message().to_owned())),
            Ok(module) => {
                let mut validator = naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::empty(),
                );
                validator.validate(&module).err().map(|e| {
                    // Its innermost place: the last span said.
                    let at = e.spans().last().map(|(span, _)| span.location(&text));
                    let mut message = e.as_inner().to_string();
                    let mut source = std::error::Error::source(e.as_inner());
                    while let Some(cause) = source {
                        message = format!("{message}: {cause}");
                        source = cause.source();
                    }
                    (at, message)
                })
            }
        };
        let Some((at, message)) = found else {
            return Ok(());
        };
        let mut message = message.replace(if layer { "_zgxl0" } else { "_zgxm" }, "");
        // Time reached another way than `frame.time` (its fields aren't
        // there without `.animated()`).
        if !self.animated
            && ["`time`", "`delta`", "`index`"]
                .iter()
                .any(|f| message.contains(f))
        {
            message.push_str(" (frame.time, .delta and .index need `.animated()`)");
        }
        let lines = self.code.lines().count() as u32;
        match at {
            Some(at) if at.line_number >= start && at.line_number < start + lines => {
                let line = at.line_number - start + 1;
                let shift = custom.shift(line, at.line_position);
                Err(error(
                    line,
                    at.line_position.saturating_sub(shift).max(1),
                    message,
                ))
            }
            // In what calls it: its function's signature isn't the one asked.
            Some(at) if at.line_number >= start + lines => {
                let (name, signature) = if custom.code[..]
                    .lines()
                    .nth((at.line_number - engine_lines - 1) as usize)
                    .is_some_and(|l| l.contains("displace"))
                {
                    ("displace", "fn displace(v: VertexIn) -> Displaced")
                } else if layer || !self.opacity {
                    ("surface", "fn surface(s: SurfaceIn) -> Surface")
                } else {
                    ("opacity", "fn opacity(s: SurfaceIn) -> f32")
                };
                Err(error(
                    line_of_fn(&self.code, name),
                    1,
                    format!("`{name}` must be `{signature}`: {message}"),
                ))
            }
            _ => Err(error(0, 0, message)),
        }
    }

    /// Its value of parameter `name` set: whether it changed, or why not.
    pub(crate) fn set(&self, name: &str, value: ParamValue) -> Result<bool, String> {
        let slot = self
            .slots
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("shader {} has no parameter `{name}`", self.id))?;
        let mut values = self.values.borrow_mut();
        let numbers: &[f32] = match (&value, slot.kind) {
            (ParamValue::Float(v), Param::Float) => std::slice::from_ref(v),
            (ParamValue::Vec2(v), Param::Vec2) => v,
            (ParamValue::Vec3(v), Param::Vec3) => v,
            (ParamValue::Vec4(v), Param::Vec4 | Param::Color) => v,
            (ParamValue::Texture(t), Param::Texture | Param::DataTexture) => {
                let held = &mut values.textures[slot.at as usize];
                if held.as_ref() == Some(t) {
                    return Ok(false);
                }
                *held = Some(t.clone());
                values.pictures += 1;
                return Ok(true);
            }
            _ => return Err(format!("`{name}` is a {:?}, not {value:?}", slot.kind)),
        };
        let bytes: &[u8] = bytemuck::cast_slice(numbers);
        let at = slot.at as usize;
        if values.bytes[at..at + bytes.len()] == *bytes {
            return Ok(false);
        }
        values.bytes[at..at + bytes.len()].copy_from_slice(bytes);
        values.numbers += 1;
        Ok(true)
    }
}

/// A world's shaders (`World::add_shader`, by id from 1) and its surface
/// layers, in order.
#[derive(Default)]
pub(crate) struct Shaders {
    pub(crate) list: Vec<std::rc::Rc<Shader>>,
    pub(crate) layers: Vec<std::rc::Rc<Shader>>,
}
impl Shaders {
    /// Whether it has none of either.
    pub(crate) fn is_empty(&self) -> bool {
        self.list.is_empty() && self.layers.is_empty()
    }
    /// Whether any of its shaders moves its meshes' vertices.
    pub(crate) fn displacing(&self) -> bool {
        self.list.iter().any(|s| s.displace)
    }
}

/// What custom code a pipeline compiles with: a mesh's shader, the world's
/// surface layers, and the functions the engine calls them by.
pub(crate) struct Custom {
    pub(crate) code: String,
    /// The mesh's shader cuts itself out (`opacity`).
    pub(crate) opacity: bool,
    /// Its mesh's shader or a layer reads time (`.animated()`): only then
    /// is the frame's time named `frame.time`.
    pub(crate) animated: bool,
    /// The line of `code` its mesh's shader's (else its first layer's) text
    /// starts at, from 0.
    user_line: u32,
    /// Columns its first user's renaming moved, by line: (line, column
    /// after, bytes added).
    edits: Vec<(u32, u32, u32)>,
}
impl Custom {
    /// The code for `mesh`'s shader (if any) under `layers`.
    pub(crate) fn new<'a>(mesh: Option<&Shader>, layers: impl Iterator<Item = &'a Shader>) -> Self {
        use std::fmt::Write;
        let layers: Vec<&Shader> = layers.collect();
        let mut code = String::new();
        // The layers' numbers, in the frame (`frame.layers.l<i>`).
        let _ = write!(code, "struct ZgxLayers {{\n    zgx_pad: vec4<f32>,\n");
        for (i, layer) in layers.iter().enumerate() {
            if layer.uniform_size > 0 {
                let _ = writeln!(code, "    l{i}: ZgxParams_zgxl{i},");
            }
        }
        code.push_str("}\n");
        let mut user_line = None;
        let mut edits = Vec::new();
        let mut put = |code: &mut String,
                       shader: &Shader,
                       suffix: &str,
                       params: &str,
                       textures: &[(String, String)]| {
            if shader.uniform_size > 0 {
                let _ = writeln!(code, "struct ZgxParams{suffix} {{");
                for slot in shader.slots.iter() {
                    if let Some((ty, _, _)) = slot.kind.layout() {
                        let _ = writeln!(code, "    {}: {ty},", slot.name);
                    }
                }
                code.push_str("}\n");
            }
            let line = code.matches('\n').count() as u32;
            let (text, moved) = rename(&shader.code, &shader.names, suffix, params, textures);
            if user_line.is_none() {
                user_line = Some(line);
                edits = moved;
            }
            code.push_str(&text);
            if !text.ends_with('\n') {
                code.push('\n');
            }
        };
        if let Some(shader) = mesh {
            let _ = writeln!(
                code,
                "@group(1) @binding({SAMPLER_BINDING}) var material_sampler: sampler;"
            );
            if shader.uniform_size > 0 {
                let _ = writeln!(
                    code,
                    "@group(1) @binding({UNIFORM_BINDING}) var<uniform> params_zgxm: ZgxParams_zgxm;"
                );
            }
            let textures: Vec<(String, String)> = shader
                .slots
                .iter()
                .filter(|s| s.kind.layout().is_none())
                .map(|s| (s.name.clone(), format!("params_zgxm_{}", s.name)))
                .collect();
            for (slot, (_, var)) in shader
                .slots
                .iter()
                .filter(|s| s.kind.layout().is_none())
                .zip(&textures)
            {
                let _ = writeln!(
                    code,
                    "@group(1) @binding({}) var {var}: texture_2d<f32>;",
                    TEXTURE_BINDING + slot.at
                );
            }
            if shader.reads_scene {
                let _ = writeln!(
                    code,
                    "@group(1) @binding({SCENE_BINDING}) var zgx_scene: texture_2d<f32>;\n\
                     @group(1) @binding({}) var zgx_scene_sampler: sampler;\n\
                     fn scene_color(uv: vec2<f32>, lod: f32) -> vec3<f32> {{\n    \
                     let c = textureSampleLevel(zgx_scene, zgx_scene_sampler, uv, lod).rgb;\n    \
                     return select(c, zgx_decode(c), frame.params.y > 0.5);\n}}",
                    SCENE_BINDING + 1
                );
            }
            put(&mut code, shader, "_zgxm", "params_zgxm", &textures);
        }
        for (i, layer) in layers.iter().enumerate() {
            put(
                &mut code,
                layer,
                &format!("_zgxl{i}"),
                &format!("frame.layers.l{i}"),
                &[],
            );
        }
        // What the engine calls.
        code.push_str("fn zgx_displace(v: VertexIn) -> Displaced {\n");
        match mesh {
            Some(shader) if shader.displace => code.push_str("    return displace_zgxm(v);\n"),
            _ => code.push_str("    return Displaced(v.position, v.normal);\n"),
        }
        code.push_str(
            "}\nfn zgx_surface(s: SurfaceIn, lit: bool) -> Surface {\n    var m = zgx_so_far;\n",
        );
        if mesh.is_some_and(|s| s.surface) {
            code.push_str("    m = surface_zgxm(s);\n    zgx_so_far = m;\n");
        }
        if !layers.is_empty() {
            code.push_str("    if lit {\n");
            for i in 0..layers.len() {
                let _ = writeln!(
                    code,
                    "        m = surface_zgxl{i}(s);\n        zgx_so_far = m;"
                );
            }
            code.push_str("    }\n");
        }
        code.push_str("    return m;\n}\n");
        let opacity = mesh.is_some_and(|s| s.opacity);
        if opacity {
            code.push_str(
                "fn zgx_opacity(s: SurfaceIn) -> f32 {\n    return opacity_zgxm(s);\n}\n",
            );
        }
        let animated = mesh.is_some_and(|s| s.animated) || layers.iter().any(|l| l.animated);
        Self {
            code,
            opacity,
            animated,
            user_line: user_line.unwrap_or(0),
            edits,
        }
    }
    /// No code, the engine's custom lines on: what a shader is checked
    /// against.
    pub(crate) fn bare(opacity: bool, animated: bool) -> Self {
        Self {
            code: String::new(),
            opacity,
            animated,
            user_line: 0,
            edits: Vec::new(),
        }
    }
    /// Bytes renaming added before `column` of the first user's `line`.
    fn shift(&self, line: u32, column: u32) -> u32 {
        self.edits
            .iter()
            .filter(|(l, after, _)| *l + 1 == line && *after < column)
            .map(|(_, _, added)| added)
            .sum()
    }
}

/// Where each layer's parameters sit in the frame's `ZgxLayers` (after its
/// 16 bytes of padding), and its size: as WGSL lays a uniform out.
pub(crate) fn layer_offsets<'a>(layers: impl Iterator<Item = &'a Shader>) -> (Vec<u32>, u32) {
    let mut offset = 16;
    let offsets = layers
        .map(|layer| {
            let at = offset;
            offset += layer.uniform_size;
            at
        })
        .collect();
    (offsets, offset)
}

/// `source` with its version line blanked, if it says `#zgx material 1`.
fn version(source: &str) -> Result<String, ShaderError> {
    let mut out = String::with_capacity(source.len());
    let mut seen = false;
    for (i, line) in source.lines().enumerate() {
        let text = line.trim();
        if !seen && !text.is_empty() && !text.starts_with("//") {
            let words: Vec<&str> = text.split_whitespace().collect();
            match words[..] {
                ["#zgx", "material", v] if v == MATERIAL_VERSION.to_string() => {}
                ["#zgx", "material", v] => {
                    return Err(error(
                        i as u32 + 1,
                        1,
                        format!("`#zgx material {v}`: this zgx reads version {MATERIAL_VERSION}"),
                    ));
                }
                _ => {
                    return Err(error(
                        i as u32 + 1,
                        1,
                        format!("a material shader starts `#zgx material {MATERIAL_VERSION}`"),
                    ));
                }
            }
            seen = true;
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !seen {
        return Err(error(1, 1, "a material shader starts `#zgx material 1`"));
    }
    Ok(out)
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with("zgx")
        && !name.starts_with("__")
}

/// A token of WGSL: an identifier, with its place (line, column, from 1;
/// byte offset), what came before it, and what follows.
struct Token<'a> {
    text: &'a str,
    line: u32,
    column: u32,
    start: usize,
    /// It follows `.` (a member or swizzle) or `@` (an attribute).
    after_dot: bool,
    after_at: bool,
}
/// The identifiers of `code` outside comments and numbers, with the brace
/// depth each is at and whether it's a struct's member (`name:` in a
/// struct's body).
fn identifiers(code: &str) -> Vec<(Token<'_>, u32, bool)> {
    let bytes = code.as_bytes();
    let (mut i, mut line, mut line_start) = (0, 1u32, 0usize);
    let mut depth = 0u32;
    let mut out = Vec::new();
    let mut last = b' ';
    // The depth a struct's body is at, while in one.
    let mut struct_body: Option<u32> = None;
    let mut struct_next = false;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\n' {
            line += 1;
            line_start = i + 1;
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            let mut nest = 0;
            while i < bytes.len() {
                if bytes[i..].starts_with(b"/*") {
                    nest += 1;
                    i += 2;
                } else if bytes[i..].starts_with(b"*/") {
                    nest -= 1;
                    i += 2;
                    if nest == 0 {
                        break;
                    }
                } else {
                    if bytes[i] == b'\n' {
                        line += 1;
                        line_start = i + 1;
                    }
                    i += 1;
                }
            }
            continue;
        }
        if c.is_ascii_digit() {
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'_')
            {
                i += 1;
            }
            last = b'0';
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let text = &code[start..i];
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let member = struct_body == Some(depth)
                && j < bytes.len()
                && bytes[j] == b':'
                && !bytes[j..].starts_with(b"::");
            if text == "struct" {
                struct_next = true;
            }
            out.push((
                Token {
                    text,
                    line,
                    column: (start - line_start) as u32 + 1,
                    start,
                    after_dot: last == b'.',
                    after_at: last == b'@',
                },
                depth,
                member,
            ));
            last = b'a';
            continue;
        }
        match c {
            b'{' => {
                depth += 1;
                if std::mem::take(&mut struct_next) {
                    struct_body = Some(depth);
                }
            }
            b'}' => {
                if struct_body == Some(depth) {
                    struct_body = None;
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        last = c;
        i += 1;
    }
    out
}
/// The names `code` declares at its top level, each with whether it's a
/// function.
fn declared(code: &str) -> Vec<(String, bool)> {
    let tokens = identifiers(code);
    let mut names = Vec::new();
    let mut next = None;
    for (token, depth, _) in &tokens {
        if *depth != 0 || token.after_at || token.after_dot {
            continue;
        }
        match (next.take(), token.text) {
            (_, "fn") => next = Some(true),
            (_, "struct" | "alias" | "const" | "override") => next = Some(false),
            (_, "var") => next = Some(false),
            // `var<uniform>`'s address space and access aren't names.
            (Some(f), "uniform" | "storage" | "private" | "workgroup" | "read" | "read_write") => {
                next = Some(f);
            }
            (Some(f), name) => names.push((name.to_owned(), f)),
            (None, _) => {}
        }
    }
    names
}
/// The line function `name` is declared at in `code` (0 if it isn't).
fn line_of_fn(code: &str, name: &str) -> u32 {
    let tokens = identifiers(code);
    tokens
        .windows(2)
        .find(|w| w[0].0.text == "fn" && w[1].0.text == name)
        .map_or(0, |w| w[1].0.line)
}
/// Each use of a name the engine gives (`scene_color`, `scene_depth`,
/// `frame`), with the member read after it, if any.
fn uses(code: &str) -> Vec<((u32, u32), &str, Option<&str>)> {
    let tokens = identifiers(code);
    let mut out = Vec::new();
    for (i, (token, _, member)) in tokens.iter().enumerate() {
        if token.after_dot || token.after_at || *member {
            continue;
        }
        if matches!(token.text, "scene_color" | "scene_depth" | "frame") {
            let next = tokens
                .get(i + 1)
                .filter(|(t, _, _)| t.after_dot)
                .map(|(t, _, _)| t.text);
            out.push(((token.line, token.column), token.text, next));
        }
    }
    out
}
/// `code` with its own top-level `names` given `suffix`, `params` as
/// `params_as`, and `params.<t>` for each texture `t` as its variable: and
/// where that moved columns (line from 0, column after, bytes added).
fn rename(
    code: &str,
    names: &[String],
    suffix: &str,
    params_as: &str,
    textures: &[(String, String)],
) -> (String, Vec<(u32, u32, u32)>) {
    let tokens = identifiers(code);
    let mut out = String::with_capacity(code.len() + 64);
    let mut edits = Vec::new();
    let mut copied = 0;
    let mut i = 0;
    while i < tokens.len() {
        let (token, _, member) = &tokens[i];
        i += 1;
        if token.after_dot || token.after_at || *member {
            continue;
        }
        let end = token.start + token.text.len();
        let replaced = if token.text == "params" {
            // `params.<texture>`: the texture's own variable.
            let texture = tokens
                .get(i)
                .filter(|(t, _, _)| t.after_dot)
                .and_then(|(t, _, _)| {
                    textures
                        .iter()
                        .find(|(name, _)| name == t.text)
                        .map(|(_, var)| (var, t.start + t.text.len()))
                });
            match texture {
                Some((var, until)) => {
                    i += 1;
                    Some((var.clone(), until))
                }
                None => Some((params_as.to_owned(), end)),
            }
        } else if names.iter().any(|n| n == token.text) {
            Some((format!("{}{suffix}", token.text), end))
        } else {
            None
        };
        if let Some((text, until)) = replaced {
            out.push_str(&code[copied..token.start]);
            out.push_str(&text);
            let removed = until - token.start;
            if text.len() > removed {
                edits.push((
                    token.line - 1,
                    token.column + removed as u32,
                    (text.len() - removed) as u32,
                ));
            }
            copied = until;
        }
    }
    out.push_str(&code[copied..]);
    (out, edits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shaders_own_names_are_renamed_and_nothing_else() {
        let code = "struct Wave { height: f32, wave: f32 }\n\
                    fn wave(p: vec2<f32>) -> Wave { var w: Wave; w.wave = p.x; return w; }\n\
                    // wave in a comment\n\
                    fn surface(s: SurfaceIn) -> Surface { let wave = wave(s.uv).wave; return surface_default(s); }";
        let names: Vec<String> = declared(code).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["Wave", "wave", "surface"]);
        let (out, _) = rename(code, &names, "_x", "p_x", &[]);
        assert!(
            out.contains("struct Wave_x { height: f32, wave: f32 }"),
            "{out}"
        );
        assert!(out.contains("fn wave_x(p: vec2<f32>) -> Wave_x { var w: Wave_x; w.wave = p.x;"));
        assert!(out.contains("// wave in a comment"));
        assert!(out.contains("let wave_x = wave_x(s.uv).wave;"));
        assert!(out.contains("fn surface_x(s: SurfaceIn) -> Surface"));
    }

    #[test]
    fn params_become_the_shaders_own_uniform_and_textures() {
        let code = "fn surface(s: SurfaceIn) -> Surface { let c = textureSample(params.foam, material_sampler, s.uv); return params.tint * c; }";
        let names = vec!["surface".to_owned()];
        let textures = vec![("foam".to_owned(), "params_zgxm_foam".to_owned())];
        let (out, edits) = rename(code, &names, "_zgxm", "params_zgxm", &textures);
        assert!(
            out.contains("textureSample(params_zgxm_foam, material_sampler"),
            "{out}"
        );
        assert!(out.contains("return params_zgxm.tint * c;"), "{out}");
        assert_eq!(edits.len(), 3);
    }

    #[test]
    fn a_shader_starts_with_its_version() {
        assert!(version("#zgx material 1\nfn f() {}").is_ok());
        assert!(version("// water\n\n  #zgx material 1\n").is_ok());
        let wrong = version("#zgx material 2\n").unwrap_err();
        assert_eq!((wrong.line, wrong.column), (1, 1));
        assert!(wrong.message.contains("version 1"), "{wrong}");
        let missing = version("\nfn f() {}\n").unwrap_err();
        assert_eq!(missing.line, 2);
    }
}
