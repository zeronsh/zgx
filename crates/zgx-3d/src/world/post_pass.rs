//! The app's own post passes (design-3d-aaa.md, 5.2): a WGSL function run
//! on every pixel of the frame, in the scene's linear light or on what the
//! display shows, with parameters the app sets by name.
use super::World;
use std::sync::Arc;
use zgx_gpu::wgpu::naga;

/// Where in the chain a post pass runs (`PostPass::stage`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PostStage {
    /// On the scene's light: linear, unbounded, before exposure, bloom and
    /// tone mapping (bloom spreads what it makes bright).
    #[default]
    Hdr,
    /// On what the display shows: after exposure, tone mapping and grading,
    /// linear light from 0 to 1, before it's encoded.
    Display,
}

/// The type of a pass's parameter (`PostPass::param`): its WGSL type in
/// `params.<name>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Param {
    /// `f32`.
    Float,
    /// `vec2<f32>`.
    Vec2,
    /// `vec3<f32>`.
    Vec3,
    /// `vec4<f32>`.
    Vec4,
}
impl Param {
    fn wgsl(self) -> &'static str {
        match self {
            Self::Float => "f32",
            Self::Vec2 => "vec2<f32>",
            Self::Vec3 => "vec3<f32>",
            Self::Vec4 => "vec4<f32>",
        }
    }
}

/// A parameter's value (`World::set_post_param`): a number or 2, 3 or 4.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamValue(pub(crate) [f32; 4], pub(crate) Param);
impl From<f32> for ParamValue {
    fn from(v: f32) -> Self {
        Self([v, 0., 0., 0.], Param::Float)
    }
}
impl From<[f32; 2]> for ParamValue {
    fn from([x, y]: [f32; 2]) -> Self {
        Self([x, y, 0., 0.], Param::Vec2)
    }
}
impl From<[f32; 3]> for ParamValue {
    fn from([x, y, z]: [f32; 3]) -> Self {
        Self([x, y, z, 0.], Param::Vec3)
    }
}
impl From<[f32; 4]> for ParamValue {
    fn from(v: [f32; 4]) -> Self {
        Self(v, Param::Vec4)
    }
}

/// A post pass of the app's (`World::add_post`): its WGSL defines
///
/// ```wgsl
/// #zgx post 1
/// fn post(p: PostIn) -> vec4<f32> {
///     return color(p.uv) * params.strength;
/// }
/// ```
///
/// given `PostIn` (`uv` from 0 to 1, `size` and `position` in pixels),
/// `color(uv)` (the frame so far, premultiplied, sampled linearly),
/// `frame` (`time` and `delta` in seconds, `index`) and `params`, a field
/// for each `param` it declares. It returns the pixel, premultiplied.
///
/// **Promise** (version 1): `PostIn`, `color`, `frame`, `params`, the two
/// stages, and that an HDR-stage pass receives the scene's linear light
/// before exposure. Not promised: bind group numbers, the order of other
/// definitions, the stock passes' targets. `depth(uv)` and `velocity(uv)`
/// come with the depth prepass.
#[derive(Clone, Debug)]
pub struct PostPass {
    source: Arc<str>,
    stage: PostStage,
    params: Vec<(String, Param)>,
    animated: bool,
}
impl PostPass {
    /// A pass from `source`, run in the HDR stage.
    pub fn new(source: impl Into<Arc<str>>) -> Self {
        Self {
            source: source.into(),
            stage: PostStage::Hdr,
            params: Vec::new(),
            animated: false,
        }
    }
    /// Run it at `stage`. Passes of a stage run in the order added.
    pub fn stage(mut self, stage: PostStage) -> Self {
        self.stage = stage;
        self
    }
    /// It changes with `frame.time`: while it's in the chain, the world
    /// draws a frame every frame its view is shown (others draw only as the
    /// world changes).
    pub fn animated(mut self) -> Self {
        self.animated = true;
        self
    }
    /// A parameter, `params.<name>` in its WGSL, 0 until set.
    pub fn param(mut self, name: &str, ty: Param) -> Self {
        self.params.push((name.into(), ty));
        self
    }
}

/// The most parameters a post pass declares (16 bytes each, in one
/// uniform buffer).
pub const MAX_POST_PARAMS: usize = 64;

/// A post pass added to a world (`World::add_post`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PostPassId(u64);

/// Why a post pass was refused, or a parameter not set.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostError {
    /// It isn't version 1's (`#zgx post 1` its first line).
    Version,
    /// Its WGSL doesn't parse or validate: naga's message, at the pass's
    /// own lines.
    Shader(String),
    /// A parameter's name isn't an identifier, is declared twice, isn't the
    /// pass's, or is set with another type than declared; or it declares
    /// more than `MAX_POST_PARAMS`.
    Param(String),
    /// No such pass (removed, or another world's).
    Pass,
}
impl std::fmt::Display for PostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Version => f.write_str("a post pass begins `#zgx post 1`"),
            Self::Shader(message) => write!(f, "the post pass's shader is invalid:\n{message}"),
            Self::Param(why) => f.write_str(why),
            Self::Pass => f.write_str("no such post pass"),
        }
    }
}
impl std::error::Error for PostError {}

impl super::Settle {
    /// An animated post pass is in the chain (`PostPass::animated`).
    pub(crate) const POST: u32 = 1 << 8;
}

/// A pass as the world keeps it: its WGSL whole (prelude and all), and its
/// parameters' values.
#[derive(Clone, Debug)]
pub(crate) struct AppPass {
    pub(crate) id: PostPassId,
    pub(crate) stage: PostStage,
    pub(crate) wgsl: Arc<str>,
    pub(crate) params: Vec<(String, Param)>,
    pub(crate) values: Vec<[f32; 4]>,
    /// Its WGSL reads `frame.time`: the world keeps drawing while it's in.
    pub(crate) animated: bool,
}

/// What every pass's WGSL begins with (a pass's lines are counted after
/// it).
fn prelude(params: &[(String, Param)]) -> String {
    let mut fields = String::new();
    for (name, ty) in params {
        fields += &format!("    @align(16) {name}: {},\n", ty.wgsl());
    }
    if params.is_empty() {
        fields += "    @align(16) zgx_none: f32,\n";
    }
    format!(
        "struct PostIn {{ uv: vec2<f32>, size: vec2<f32>, position: vec2<f32> }}
struct ZgxFrame {{ time: f32, delta: f32, index: u32, zgx_pad: u32 }}
struct ZgxParams {{
{fields}}}
@group(0) @binding(0) var zgx_input: texture_2d<f32>;
@group(0) @binding(1) var zgx_sampler: sampler;
@group(0) @binding(2) var<uniform> frame: ZgxFrame;
@group(0) @binding(3) var<uniform> params: ZgxParams;
fn color(uv: vec2<f32>) -> vec4<f32> {{ return textureSampleLevel(zgx_input, zgx_sampler, uv, 0.0); }}
struct ZgxOut {{ @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }}
@vertex fn zgx_vertex(@builtin(vertex_index) i: u32) -> ZgxOut {{
    let uv = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return ZgxOut(vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0), uv);
}}
override ZGX_ENCODE: bool = false;
fn zgx_encode(c: vec3<f32>) -> vec3<f32> {{
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3(0.0031308));
}}
@fragment fn zgx_fragment(o: ZgxOut) -> @location(0) vec4<f32> {{
    let size = vec2<f32>(textureDimensions(zgx_input));
    var c = post(PostIn(o.uv, size, o.position.xy));
    if ZGX_ENCODE {{
        let a = max(c.a, 1e-4);
        c = vec4(zgx_encode(clamp(c.rgb / a, vec3(0.0), vec3(1.0))) * c.a, c.a);
    }}
    return c;
}}
"
    )
}

/// `pass` checked and made whole, or why it can't be.
fn check(pass: &PostPass) -> Result<Arc<str>, PostError> {
    let mut lines = pass.source.splitn(2, '\n');
    if lines.next().map(str::trim) != Some("#zgx post 1") {
        return Err(PostError::Version);
    }
    let body = lines.next().unwrap_or("");
    if pass.params.len() > MAX_POST_PARAMS {
        return Err(PostError::Param(format!(
            "{} parameters, more than {MAX_POST_PARAMS}",
            pass.params.len()
        )));
    }
    for (i, (name, _)) in pass.params.iter().enumerate() {
        let identifier = name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with("zgx");
        if !identifier {
            return Err(PostError::Param(format!(
                "`{name}` isn't a parameter's name"
            )));
        }
        if pass.params[..i].iter().any(|(other, _)| other == name) {
            return Err(PostError::Param(format!("`{name}` is declared twice")));
        }
    }
    let prelude = prelude(&pass.params);
    // The directive's line kept as a blank one: the app's lines keep their
    // numbers after the prelude's.
    let whole = format!("{prelude}\n{body}");
    let skipped = prelude.lines().count();
    let at_own_lines = |message: String| {
        // naga's report names `wgsl:LINE:COLUMN`: the prelude's lines off.
        let mut out = String::new();
        let mut rest = message.as_str();
        while let Some(at) = rest.find("wgsl:") {
            out += &rest[..at + 5];
            rest = &rest[at + 5..];
            let digits = rest.chars().take_while(char::is_ascii_digit).count();
            match rest[..digits].parse::<usize>() {
                Ok(line) if line > skipped => out += &(line - skipped).to_string(),
                _ => out += &rest[..digits],
            }
            rest = &rest[digits..];
        }
        out + rest
    };
    let module = naga::front::wgsl::parse_str(&whole)
        .map_err(|e| PostError::Shader(at_own_lines(e.emit_to_string(&whole))))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| PostError::Shader(at_own_lines(e.emit_to_string(&whole))))?;
    Ok(whole.into())
}

impl World {
    /// Add `pass` to its post chain, after the passes of its stage already
    /// there; the chain is on while it has one. Checked now: an invalid
    /// shader is refused with naga's message at the pass's own lines.
    pub fn add_post(&self, pass: PostPass) -> Result<PostPassId, PostError> {
        let wgsl = check(&pass)?;
        let mut id = None;
        self.change_view(|inner| {
            inner.post_ids += 1;
            let made = PostPassId(inner.post_ids);
            id = Some(made);
            inner.post_passes.push(AppPass {
                id: made,
                stage: pass.stage,
                wgsl,
                values: vec![[0.; 4]; pass.params.len()],
                params: pass.params,
                animated: pass.animated,
            });
            true
        });
        self.animate_posts();
        Ok(id.expect("added"))
    }
    /// Its view kept drawing while an animated pass is in.
    fn animate_posts(&self) {
        let inner = self.inner.borrow();
        let on = inner.post_passes.iter().any(|p| p.animated);
        inner.unsettle(super::Settle::POST, on);
    }
    /// Take `pass` out of its post chain.
    pub fn remove_post(&self, pass: PostPassId) {
        self.change_view(|inner| {
            let before = inner.post_passes.len();
            inner.post_passes.retain(|p| p.id != pass);
            inner.post_passes.len() != before
        });
        self.animate_posts();
    }
    /// Set `pass`'s parameter `name` to `value` (of the type it declared).
    pub fn set_post_param(
        &self,
        pass: PostPassId,
        name: &str,
        value: impl Into<ParamValue>,
    ) -> Result<(), PostError> {
        let ParamValue(value, ty) = value.into();
        let mut result = Ok(());
        self.change_view(|inner| {
            let Some(found) = inner.post_passes.iter_mut().find(|p| p.id == pass) else {
                result = Err(PostError::Pass);
                return false;
            };
            let Some(at) = found.params.iter().position(|(n, _)| n == name) else {
                result = Err(PostError::Param(format!(
                    "the pass has no parameter `{name}`"
                )));
                return false;
            };
            if found.params[at].1 != ty {
                result = Err(PostError::Param(format!(
                    "`{name}` is a {:?}, not a {ty:?}",
                    found.params[at].1
                )));
                return false;
            }
            std::mem::replace(&mut found.values[at], value) != value
        });
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pass_is_checked_as_its_added_and_its_errors_are_at_its_own_lines() {
        let fine = PostPass::new("#zgx post 1\nfn post(p: PostIn) -> vec4<f32> {\n    return color(p.uv) * params.k;\n}\n")
            .param("k", Param::Float);
        assert!(check(&fine).is_ok());
        assert_eq!(
            check(&PostPass::new(
                "fn post(p: PostIn) -> vec4<f32> { return vec4(0.0); }"
            )),
            Err(PostError::Version)
        );
        // An unknown name on its line 3.
        let wrong = PostPass::new(
            "#zgx post 1\nfn post(p: PostIn) -> vec4<f32> {\n    return colour(p.uv);\n}\n",
        );
        let Err(PostError::Shader(message)) = check(&wrong) else {
            panic!("refused");
        };
        assert!(message.contains("wgsl:3:"), "{message}");
        assert!(matches!(
            check(&fine.clone().param("k", Param::Vec2)),
            Err(PostError::Param(_))
        ));
        assert!(matches!(
            check(&PostPass::new("#zgx post 1\n").param("2x", Param::Float)),
            Err(PostError::Param(_))
        ));
        // At most MAX_POST_PARAMS (a uniform buffer's binding is bounded).
        let mut many =
            PostPass::new("#zgx post 1\nfn post(p: PostIn) -> vec4<f32> { return color(p.uv); }\n");
        for i in 0..=MAX_POST_PARAMS {
            many = many.param(&format!("p{i}"), Param::Vec4);
        }
        assert!(matches!(check(&many), Err(PostError::Param(_))));
    }
}
