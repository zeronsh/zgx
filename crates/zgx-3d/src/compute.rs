//! Compute passes of the app's (design-3d-aaa.md, 5.3): WGSL compute shaders
//! with buffers and textures bound by name, each run every frame, at a rate
//! or on demand, at one of three points in the frame.
//!
//! A pass's shader is parsed and checked by naga when it's added, against
//! WebGPU's default limits (what every tier A device meets, and what the
//! shared device asks for): its errors carry the app's own line numbers, and
//! nothing the frame later does with it can fail on what was checked. Its
//! pipeline is compiled on a thread of its own with the first frame on a
//! device; the pass runs from the first frame after it's ready.
//!
//! Passes at a point run in the order added, except that a pass writing a
//! resource whole (`Access::Write`) runs before passes added earlier that
//! only read it. An order that can't hold is refused when the pass is added.
//!
//! A world with an enabled every-frame or rated pass keeps drawing (it's
//! unsettled); one with on-demand passes draws when one is asked to run.
use crate::{
    buffer::{
        BufferShared, Chunk, DEFAULT_MEMORY_CAP, DEFAULT_READBACK_BUDGET, Format, GpuBuffer,
        GpuTexture, Link, ReadError, Source, Stage, TextureShared,
    },
    world::Inner,
};
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    time::Instant,
};
use zgx_gpu::{
    viewport::ViewportFrame,
    wgpu::{self, naga},
};

/// The `Settle` reason of a world's compute: passes due, reads on their way.
pub(crate) const UNSETTLED: u32 = 1 << 8;

/// How a pass uses a resource it binds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Access {
    /// It reads it: `var<storage, read>`, `var<uniform>`, a sampled texture
    /// or `texture_storage_2d<…, read>`.
    Read,
    /// It writes it whole, reading none of it: `var<storage, read_write>`
    /// or `texture_storage_2d<…, write>`. It runs before passes added
    /// earlier that read it.
    Write,
    /// It reads and writes it: `var<storage, read_write>` or
    /// `texture_storage_2d<…, read_write>` (one-channel 32-bit formats).
    ReadWrite,
}
impl Access {
    fn writes(self) -> bool {
        self != Self::Read
    }
}

/// Where in the frame a pass runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Point {
    /// Before anything is drawn: what the frame draws reads its results.
    #[default]
    BeforeFrame,
    /// After the opaque meshes, before the translucent ones and particles.
    /// Not yet: refused until the renderer's pass list has it.
    BeforeTransparents,
    /// After everything is drawn: what the next frame reads, and readbacks.
    AfterFrame,
}
impl Point {
    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Run {
    EveryFrame,
    OnDemand,
    AtRate(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dispatch {
    Workgroups([u32; 3]),
    Threads([u32; 3]),
}

#[derive(Clone)]
enum Bound {
    Buffer(GpuBuffer),
    Texture(GpuTexture),
}

/// How much work compute passes may give the device
/// (`World::set_compute_budget`). The device is shared with every window's
/// interface, and WGSL's loops can't be bounded by reading the shader: a
/// pass is held to these where it runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputeBudget {
    /// Invocations a frame's passes may run, all told (2^26 to begin with).
    /// A pass with more alone is refused when added; passes past it in a
    /// frame wait for the next.
    pub invocations: u64,
    /// GPU milliseconds a pass may take, measured where the device has
    /// timestamps (8 to begin with).
    pub gpu_ms: f32,
    /// The same for a sandboxed pass (`Compute::sandboxed`; 1 to begin
    /// with).
    pub sandboxed_gpu_ms: f32,
    /// GPU milliseconds an owner's sandboxed passes may take a frame
    /// together (2 to begin with): over it as often as `frames` says, all
    /// of them are turned off.
    pub owner_gpu_ms: f32,
    /// Of a pass's last 8 timed frames, how many may take longer before it's
    /// turned off (`ComputeStatus::OverBudget`; 3 to begin with): one slow
    /// on alternate frames trips it as one slow every frame does. Frames
    /// not timed (more than four frames' timestamps on their way) count
    /// neither way.
    pub frames: u32,
}
impl Default for ComputeBudget {
    fn default() -> Self {
        Self {
            invocations: 1 << 26,
            gpu_ms: 8.,
            sandboxed_gpu_ms: 1.,
            owner_gpu_ms: 2.,
            frames: 3,
        }
    }
}

/// A compute pass to add (`World::add_compute`): WGSL, the resources it
/// binds by the names its shader gives them, how many workgroups it runs,
/// when and where in the frame.
///
/// ```wgsl
/// struct Frame { time: f32, delta: f32, index: u32 }
/// @group(0) @binding(0) var<uniform> frame: Frame;          // bound by zgx-3d
/// @group(0) @binding(1) var<storage, read_write> waves: array<vec4f>;
/// @group(0) @binding(2) var height: texture_storage_2d<rgba16float, write>;
///
/// @compute @workgroup_size(8, 8)
/// fn step(@builtin(global_invocation_id) id: vec3u) { … }
/// ```
///
/// A uniform named `frame` gets the frame's time in seconds since the
/// world's first frame, the step since the last, and a counter; a `sampler`
/// gets a linear one, clamped at the edges. Bindings the entry point doesn't
/// use are left out.
#[derive(Clone)]
pub struct Compute {
    source: Arc<str>,
    entry: Option<String>,
    binds: Vec<(String, Bound, Access)>,
    dispatch: Dispatch,
    run: Run,
    point: Point,
    /// Who it's run for, where the app doesn't vouch for it (a plugin).
    sandboxed: Option<Arc<str>>,
}
impl std::fmt::Debug for Compute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compute")
            .field("entry", &self.entry)
            .field("dispatch", &self.dispatch)
            .field("run", &self.run)
            .field("point", &self.point)
            .finish()
    }
}
impl Compute {
    /// A pass running `wgsl`: run once when added and again when asked
    /// (`on_demand`), before the frame, one workgroup.
    pub fn new(wgsl: impl Into<Arc<str>>) -> Self {
        Self {
            source: wgsl.into(),
            entry: None,
            binds: Vec::new(),
            dispatch: Dispatch::Workgroups([1, 1, 1]),
            run: Run::OnDemand,
            point: Point::BeforeFrame,
            sandboxed: None,
        }
    }
    /// Its entry point, where the shader has more than one.
    pub fn entry(mut self, name: &str) -> Self {
        self.entry = Some(name.to_owned());
        self
    }
    /// `buffer` bound as the shader's `name`, used as `access` says.
    pub fn buffer(mut self, name: &str, buffer: &GpuBuffer, access: Access) -> Self {
        self.binds
            .push((name.to_owned(), Bound::Buffer(buffer.clone()), access));
        self
    }
    /// `texture` bound as the shader's `name`, used as `access` says.
    pub fn texture(mut self, name: &str, texture: &GpuTexture, access: Access) -> Self {
        self.binds
            .push((name.to_owned(), Bound::Texture(texture.clone()), access));
        self
    }
    /// Runs this many workgroups along each axis.
    pub fn dispatch(mut self, workgroups: [u32; 3]) -> Self {
        self.dispatch = Dispatch::Workgroups(workgroups);
        self
    }
    /// Runs enough workgroups for this many invocations along each axis (by
    /// the shader's `@workgroup_size`, rounded up).
    pub fn threads(mut self, threads: [u32; 3]) -> Self {
        self.dispatch = Dispatch::Threads(threads);
        self
    }
    /// Runs every frame while enabled: the world keeps drawing.
    pub fn every_frame(mut self) -> Self {
        self.run = Run::EveryFrame;
        self
    }
    /// Runs once when added, then once each `World::run_compute`.
    pub fn on_demand(mut self) -> Self {
        self.run = Run::OnDemand;
        self
    }
    /// Runs `per_second` times a second of frame time while enabled, at most
    /// once a frame. The world draws while it's due within two frames and
    /// rests between: until zgx can wake a view at a time, a pass slower
    /// than the display runs with the first frame drawn (for any reason) at
    /// or after its due time.
    pub fn at_rate(mut self, per_second: f32) -> Self {
        self.run = Run::AtRate(per_second);
        self
    }
    /// Where in the frame it runs.
    pub fn at(mut self, point: Point) -> Self {
        self.point = point;
        self
    }
    /// Code the app doesn't vouch for, run for `owner` (a plugin's name):
    /// checked so it can't hold the device (its size, memory and cost
    /// bounded, its loops capped: `shader_sandbox`), held to the stricter
    /// GPU time budgets (`ComputeBudget::sandboxed_gpu_ms`, `owner_gpu_ms`)
    /// and to its share of a frame's, and refused (`ComputeError::Untimed`)
    /// on a device that can't time it.
    pub fn sandboxed(mut self, owner: impl Into<Arc<str>>) -> Self {
        self.sandboxed = Some(owner.into());
        self
    }
}

/// A compute pass in a world (`World::add_compute`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ComputeId {
    index: u32,
    generation: u32,
}

/// Why a compute pass was refused.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComputeError {
    /// The device runs no compute shaders (tier B: `World::caps`).
    Unsupported,
    /// Its WGSL doesn't parse or validate: naga's message, at the app's
    /// own lines.
    Shader(String),
    /// No such entry point, or more than one and none named.
    Entry(String),
    /// A binding the shader and the pass don't agree on.
    Binding {
        /// The shader's name for it.
        name: String,
        /// What's wrong.
        why: String,
    },
    /// More than a tier A device is sure to allow.
    Limits(String),
    /// Its point in the frame isn't there yet.
    Point(Point),
    /// Its resources' order can't hold with the passes before it.
    Cycle,
    /// Sandboxed, on a device without timestamps: nothing could stop a
    /// shader that runs too long.
    Untimed,
}
impl std::fmt::Display for ComputeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => f.write_str("the device runs no compute shaders"),
            Self::Shader(message) => write!(f, "the compute shader is invalid:\n{message}"),
            Self::Entry(why) => f.write_str(why),
            Self::Binding { name, why } => write!(f, "`{name}`: {why}"),
            Self::Limits(why) => f.write_str(why),
            Self::Point(point) => write!(f, "compute passes can't run at {point:?} yet"),
            Self::Untimed => {
                f.write_str("a sandboxed pass needs a device that can time it (`Caps::timestamps`)")
            }
            Self::Cycle => f.write_str(
                "the pass writes what an earlier pass reads, which writes what it reads",
            ),
        }
    }
}
impl std::error::Error for ComputeError {}

/// Where a pass is on the device (`World::compute_status`).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum ComputeStatus {
    /// Waiting for a frame, or compiling.
    Compiling,
    /// Compiled: it runs when due.
    Ready,
    /// The device refused it, or runs no compute: why.
    Failed(String),
    /// Turned off: it took longer than its budget (`ComputeBudget`) frames
    /// running, the last this many milliseconds.
    OverBudget {
        /// Its last frame's GPU time.
        gpu_ms: f32,
    },
}

/// What a pass binds at a slot of its shader's.
#[derive(Clone)]
enum Slot {
    Buffer(GpuBuffer),
    Texture(GpuTexture),
    Frame,
    Sampler,
}

/// A resource by identity, for ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Buffer(u64),
    Texture(u64),
}

/// A pass's shader as checked: its entry, workgroups and bindings by group.
struct Checked {
    entry: String,
    workgroups: [u32; 3],
    /// Invocations a run: its workgroups times their size.
    invocations: u64,
    /// A sandboxed pass's loop iterations an invocation, all its loops
    /// together (`shader_sandbox`).
    iteration_cap: Option<u32>,
    groups: Vec<Vec<(u32, Slot)>>,
    uses: Vec<(Key, Access)>,
}

/// WebGPU's default limits, which every tier A device meets.
fn limits() -> wgpu::Limits {
    wgpu::Limits::default()
}

/// `compute` checked against its shader and the default limits.
fn check(compute: &Compute) -> Result<Checked, ComputeError> {
    let source = &*compute.source;
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| ComputeError::Shader(e.emit_to_string(source)))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| ComputeError::Shader(e.emit_to_string(source)))?;
    let computes: Vec<usize> = (0..module.entry_points.len())
        .filter(|&i| module.entry_points[i].stage == naga::ShaderStage::Compute)
        .collect();
    let names = || {
        computes
            .iter()
            .map(|&i| module.entry_points[i].name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let at = match &compute.entry {
        Some(name) => *computes
            .iter()
            .find(|&&i| module.entry_points[i].name == *name)
            .ok_or_else(|| {
                ComputeError::Entry(format!(
                    "no compute entry point `{name}` (it has: {})",
                    names()
                ))
            })?,
        None => match computes[..] {
            [one] => one,
            [] => return Err(ComputeError::Entry("no `@compute` entry point".into())),
            _ => {
                return Err(ComputeError::Entry(format!(
                    "name one of its entry points with `entry`: {}",
                    names()
                )));
            }
        },
    };
    let entry = &module.entry_points[at];
    let limits = limits();
    if entry.workgroup_size_overrides.is_some() {
        return Err(ComputeError::Limits(
            "a `@workgroup_size` set by overrides isn't supported".into(),
        ));
    }
    let size = entry.workgroup_size;
    let most = [
        limits.max_compute_workgroup_size_x,
        limits.max_compute_workgroup_size_y,
        limits.max_compute_workgroup_size_z,
    ];
    if size.iter().zip(most).any(|(s, m)| *s > m)
        || size.iter().product::<u32>() > limits.max_compute_invocations_per_workgroup
    {
        return Err(ComputeError::Limits(format!(
            "a workgroup of {size:?} is more than {most:?} or {} invocations",
            limits.max_compute_invocations_per_workgroup
        )));
    }
    let workgroups = match compute.dispatch {
        Dispatch::Workgroups(w) => w,
        Dispatch::Threads(t) => [0, 1, 2].map(|i| t[i].div_ceil(size[i].max(1))),
    };
    if workgroups
        .iter()
        .any(|&w| w > limits.max_compute_workgroups_per_dimension)
    {
        return Err(ComputeError::Limits(format!(
            "{workgroups:?} workgroups is more than {} along an axis",
            limits.max_compute_workgroups_per_dimension
        )));
    }

    let used = info.get_entry_point(at);
    let bound = |name: &str| compute.binds.iter().find(|(n, ..)| n == name);
    let err = |name: &str, why: String| ComputeError::Binding {
        name: name.to_owned(),
        why,
    };
    for (name, ..) in &compute.binds {
        let declared = module
            .global_variables
            .iter()
            .any(|(_, g)| g.name.as_deref() == Some(name) && g.binding.is_some());
        if !declared {
            return Err(err(name, "the shader binds nothing of that name".into()));
        }
    }
    let mut groups: Vec<Vec<(u32, Slot)>> = Vec::new();
    let mut uses: Vec<(Key, Access)> = Vec::new();
    let (mut storage_buffers, mut storage_textures, mut sampled, mut uniforms) = (0, 0, 0, 0);
    for (handle, global) in module.global_variables.iter() {
        let Some(binding) = &global.binding else {
            continue;
        };
        if used[handle].is_empty() {
            continue;
        }
        let name = global.name.as_deref().unwrap_or("");
        let declared = |kind: &str| -> Result<(&Bound, Access), ComputeError> {
            bound(name).map(|(_, b, a)| (b, *a)).ok_or_else(|| {
                err(
                    name,
                    format!("the shader uses it ({kind}) and nothing is bound to it"),
                )
            })
        };
        let slot = match global.space {
            naga::AddressSpace::Uniform if name == "frame" => Slot::Frame,
            naga::AddressSpace::Uniform => {
                uniforms += 1;
                let (bound, access) = declared("a uniform buffer")?;
                let Bound::Buffer(buffer) = bound else {
                    return Err(err(name, "a uniform is a buffer, not a texture".into()));
                };
                if access != Access::Read {
                    return Err(err(
                        name,
                        "a uniform is read only: bind it with Access::Read".into(),
                    ));
                }
                if buffer.size() > u64::from(limits.max_uniform_buffer_binding_size as u32) {
                    return Err(err(
                        name,
                        format!(
                            "a uniform buffer holds at most {} bytes",
                            limits.max_uniform_buffer_binding_size
                        ),
                    ));
                }
                uses.push((Key::Buffer(buffer.0.id), access));
                Slot::Buffer(buffer.clone())
            }
            naga::AddressSpace::Storage { access: shader } => {
                storage_buffers += 1;
                let (bound, access) = declared("a storage buffer")?;
                let Bound::Buffer(buffer) = bound else {
                    return Err(err(name, "a storage buffer, not a texture".into()));
                };
                let writes = shader.contains(naga::StorageAccess::STORE);
                if writes != access.writes() {
                    return Err(err(
                        name,
                        if writes {
                            "the shader declares it `read_write`: bind it with Access::Write or ReadWrite".into()
                        } else {
                            "the shader declares it `read`: bind it with Access::Read".into()
                        },
                    ));
                }
                if buffer.size() > limits.max_storage_buffer_binding_size {
                    return Err(err(
                        name,
                        format!(
                            "a storage binding reads at most {} bytes",
                            limits.max_storage_buffer_binding_size
                        ),
                    ));
                }
                uses.push((Key::Buffer(buffer.0.id), access));
                Slot::Buffer(buffer.clone())
            }
            naga::AddressSpace::Handle => match &module.types[global.ty].inner {
                naga::TypeInner::Sampler { comparison: false } => Slot::Sampler,
                naga::TypeInner::Image {
                    dim: naga::ImageDimension::D2,
                    arrayed: false,
                    class,
                } => {
                    let (bound, access) = declared("a texture")?;
                    let Bound::Texture(texture) = bound else {
                        return Err(err(name, "a texture, not a buffer".into()));
                    };
                    match class {
                        naga::ImageClass::Storage {
                            format,
                            access: shader,
                        } => {
                            storage_textures += 1;
                            if Format::from_naga(*format) != Some(texture.format()) {
                                return Err(err(
                                    name,
                                    format!(
                                        "the shader declares {format:?}; the texture is {:?}",
                                        texture.format()
                                    ),
                                ));
                            }
                            let wants = match (
                                shader.contains(naga::StorageAccess::LOAD),
                                shader.contains(naga::StorageAccess::STORE),
                            ) {
                                (true, true) => Access::ReadWrite,
                                (false, true) => Access::Write,
                                _ => Access::Read,
                            };
                            if wants != access {
                                return Err(err(
                                    name,
                                    format!("the shader declares it for {wants:?}: bind it so"),
                                ));
                            }
                            if access == Access::ReadWrite && !texture.format().read_write() {
                                return Err(err(name, "only one-channel 32-bit textures are read and written by one pass".into()));
                            }
                        }
                        naga::ImageClass::Sampled { multi: false, .. } => {
                            sampled += 1;
                            if access != Access::Read {
                                return Err(err(
                                    name,
                                    "a sampled texture is read: bind it with Access::Read".into(),
                                ));
                            }
                        }
                        _ => return Err(err(name, "a texture of a kind passes don't bind".into())),
                    }
                    uses.push((Key::Texture(texture.0.id), access));
                    Slot::Texture(texture.clone())
                }
                _ => return Err(err(name, "a resource of a kind passes don't bind".into())),
            },
            _ => return Err(err(name, "a resource of a kind passes don't bind".into())),
        };
        let group = binding.group as usize;
        if group >= limits.max_bind_groups as usize {
            return Err(err(
                name,
                format!("groups go up to {}", limits.max_bind_groups - 1),
            ));
        }
        if groups.len() <= group {
            groups.resize_with(group + 1, Vec::new);
        }
        groups[group].push((binding.binding, slot));
    }
    for (count, most, what) in [
        (
            storage_buffers,
            limits.max_storage_buffers_per_shader_stage,
            "storage buffers",
        ),
        (
            storage_textures,
            limits.max_storage_textures_per_shader_stage,
            "storage textures",
        ),
        (
            sampled,
            limits.max_sampled_textures_per_shader_stage,
            "sampled textures",
        ),
        (
            uniforms,
            limits.max_uniform_buffers_per_shader_stage,
            "uniform buffers",
        ),
    ] {
        if count > most {
            return Err(ComputeError::Limits(format!(
                "{count} {what}: at most {most}"
            )));
        }
    }
    // One resource under two names, written by either: the device refuses it.
    for (i, (key, access)) in uses.iter().enumerate() {
        if uses[i + 1..]
            .iter()
            .any(|(k, a)| k == key && (a.writes() || access.writes()))
        {
            return Err(ComputeError::Binding {
                name: String::new(),
                why: "one resource is bound twice and written".into(),
            });
        }
    }
    Ok(Checked {
        entry: entry.name.clone(),
        workgroups,
        invocations: workgroups
            .iter()
            .chain(&size)
            .map(|&n| u64::from(n))
            .product(),
        iteration_cap: None,
        groups,
        uses,
    })
}

/// Passes `(added, uses)` in the order they run: each after those added
/// before it that share a resource it or they write, except that one writing
/// a resource whole goes before those added earlier only reading it. Ties
/// keep the order added. `None` if there's no such order.
fn order(passes: &[(u64, &[(Key, Access)])]) -> Option<Vec<usize>> {
    let n = passes.len();
    let mut by_added: Vec<usize> = (0..n).collect();
    by_added.sort_by_key(|&i| passes[i].0);
    // `before[j]` lists what must run before `j`.
    let mut before: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (x, &i) in by_added.iter().enumerate() {
        for &j in &by_added[x + 1..] {
            for &(key, a) in passes[i].1 {
                for &(_, b) in passes[j].1.iter().filter(|(k, _)| *k == key) {
                    if a == Access::Read && b == Access::Write {
                        before[i].push(j);
                    } else if a.writes() || b.writes() {
                        before[j].push(i);
                    }
                }
            }
        }
    }
    let mut placed = vec![false; n];
    let mut list = Vec::with_capacity(n);
    while list.len() < n {
        let next = by_added
            .iter()
            .copied()
            .find(|&i| !placed[i] && before[i].iter().all(|&j| placed[j]))?;
        placed[next] = true;
        list.push(next);
    }
    Some(list)
}

/// A pipeline compiled on a thread of its own: `None` until it's done.
type Compiled = Arc<Mutex<Option<Result<wgpu::ComputePipeline, String>>>>;

enum Gpu {
    /// Compiling since then, on the sandboxed worker of that generation if
    /// it's sandboxed.
    Compiling(Compiled, Instant, Option<u64>),
    Ready(wgpu::ComputePipeline, Vec<wgpu::BindGroup>),
    Failed,
}

struct Pass {
    added: u64,
    point: Point,
    run: Run,
    checked: Checked,
    source: Arc<str>,
    enabled: bool,
    /// An on-demand pass asked to run.
    demanded: bool,
    /// When a rated pass is next due, in seconds of frame time.
    due: f64,
    /// Its pipeline on the device of this generation.
    gpu: Option<(u64, Gpu)>,
    /// Why it can't run, if it can't.
    failed: Option<String>,
    /// Who it's run for, sandboxed; its run's work at most, weighed
    /// (`shader_sandbox::Sandboxed::work`).
    owner: Option<Arc<str>>,
    work: u64,
    /// Due last frame and left waiting by the frame's invocations: it's
    /// given room first this frame. And whether this frame runs it.
    waited: bool,
    chosen: bool,
    /// Which of its last 8 timed frames took longer than its budget (a bit
    /// each, the newest lowest), and the last's time once that turned it
    /// off.
    over: u8,
    over_budget: Option<f32>,
}
impl Pass {
    /// Whether it keeps the world drawing at `(time, frame)`: a rated pass
    /// only while it's due within the next two frames, `frame` seconds each
    /// (one, frames' jitter puts a pass at the display's rate just past).
    fn wants_frames(&self, (time, frame): (f64, f64)) -> bool {
        self.enabled
            && self.failed.is_none()
            && match self.run {
                Run::EveryFrame => true,
                Run::AtRate(_) => self.due - time <= 2. * frame,
                Run::OnDemand => self.demanded,
            }
    }
}

/// A world's GPU data and compute passes (`Inner::data`), made with the
/// first buffer, texture or pass it's given.
pub(crate) struct Data {
    pub(crate) link: Rc<Link>,
    buffers: Vec<Weak<BufferShared>>,
    textures: Vec<Weak<TextureShared>>,
    pub(crate) buffer_bytes: u64,
    pub(crate) texture_bytes: u64,
    /// The world's particle pools, counted under the same cap.
    pub(crate) particle_bytes: u64,
    pub(crate) cap: u64,
    pub(crate) budget: u64,
    /// What passes may give the device (`World::set_compute_budget`).
    pub(crate) limits: ComputeBudget,
    /// Each owner of sandboxed passes' last 8 timed frames over its budget.
    owners: Vec<(Arc<str>, u8)>,
    /// The last frame's time and step, in seconds: when the next frame is.
    clock: (f64, f64),
    passes: Vec<(u32, Option<Pass>)>,
    /// Each point's passes in the order they run.
    order: [Vec<u32>; 3],
    added: u64,
}
impl Data {
    pub(crate) fn new(view: zgx::gpu_view::GpuView) -> Self {
        Self {
            link: Rc::new(Link::new(view)),
            buffers: Vec::new(),
            textures: Vec::new(),
            buffer_bytes: 0,
            texture_bytes: 0,
            particle_bytes: 0,
            cap: DEFAULT_MEMORY_CAP,
            budget: DEFAULT_READBACK_BUDGET,
            limits: ComputeBudget::default(),
            owners: Vec::new(),
            clock: (0., 0.),
            passes: Vec::new(),
            order: Default::default(),
            added: 0,
        }
    }
    /// Lets go of handles dropped, and counts the bytes of those alive.
    pub(crate) fn sweep(&mut self) {
        self.buffers.retain(|b| b.strong_count() > 0);
        self.textures.retain(|t| t.strong_count() > 0);
        self.buffer_bytes = self
            .buffers
            .iter()
            .filter_map(Weak::upgrade)
            .map(|b| b.size)
            .sum();
        self.texture_bytes = self
            .textures
            .iter()
            .filter_map(Weak::upgrade)
            .map(|t| t.desc.bytes())
            .sum();
    }
    /// Room for `bytes` more under the cap.
    pub(crate) fn room(&mut self, bytes: u64) -> Result<(), crate::GpuDataError> {
        self.sweep();
        let used = self.buffer_bytes + self.texture_bytes + self.particle_bytes;
        if used.saturating_add(bytes) > self.cap {
            return Err(crate::GpuDataError::OverBudget {
                asked: bytes,
                used,
                cap: self.cap,
            });
        }
        Ok(())
    }
    pub(crate) fn add_buffer(&mut self, shared: &Rc<BufferShared>) {
        self.buffer_bytes += shared.size;
        self.buffers.push(Rc::downgrade(shared));
    }
    pub(crate) fn add_texture(&mut self, shared: &Rc<TextureShared>) {
        self.texture_bytes += shared.desc.bytes();
        self.textures.push(Rc::downgrade(shared));
    }

    /// `compute` checked and placed in its point's order.
    /// `compute` checked and placed in its point's order; `timed`, whether
    /// the device is known to have timestamps (`None`: not yet known).
    pub(crate) fn add_compute(
        &mut self,
        compute: Compute,
        timed: Option<bool>,
    ) -> Result<ComputeId, ComputeError> {
        if compute.point == Point::BeforeTransparents {
            return Err(ComputeError::Point(compute.point));
        }
        if compute.sandboxed.is_some() && timed == Some(false) {
            return Err(ComputeError::Untimed);
        }
        if let Some(owner) = &compute.sandboxed {
            let sandboxed = || {
                self.passes
                    .iter()
                    .filter_map(|(_, p)| p.as_ref()?.owner.as_ref())
            };
            if sandboxed().count() >= MOST_SANDBOXED {
                return Err(ComputeError::Limits(format!(
                    "a world holds at most {MOST_SANDBOXED} sandboxed passes"
                )));
            }
            if sandboxed().filter(|o| *o == owner).count() >= MOST_AN_OWNERS {
                return Err(ComputeError::Limits(format!(
                    "`{owner}` holds at most {MOST_AN_OWNERS} sandboxed passes"
                )));
            }
        }
        if compute.sandboxed.is_some() && compute.source.len() > MOST_SANDBOXED_SOURCE {
            return Err(ComputeError::Limits(format!(
                "a sandboxed shader of {} bytes: at most {MOST_SANDBOXED_SOURCE}",
                compute.source.len()
            )));
        }
        if self.passes.iter().filter(|(_, p)| p.is_some()).count() >= MOST_PASSES {
            return Err(ComputeError::Limits(format!(
                "a world holds at most {MOST_PASSES} compute passes"
            )));
        }
        let mut checked = check(&compute)?;
        let mut source = compute.source.clone();
        let mut work = 0;
        if compute.sandboxed.is_some() {
            if checked.invocations > MOST_SANDBOXED_INVOCATIONS {
                return Err(ComputeError::Limits(format!(
                    "{} invocations: a sandboxed pass runs at most {MOST_SANDBOXED_INVOCATIONS}",
                    checked.invocations
                )));
            }
            // Its size, memory and cost checked, its loops capped so a run's
            // work stays within the sandbox's (`shader_sandbox`).
            let module = naga::front::wgsl::parse_str(&compute.source)
                .map_err(|e| ComputeError::Shader(e.emit_to_string(&compute.source)))?;
            let made = crate::shader_sandbox::sandbox(
                module,
                &checked.entry,
                checked.invocations,
                &crate::shader_sandbox::Limits::default(),
            )
            .map_err(ComputeError::Limits)?;
            let written =
                crate::shader_sandbox::to_wgsl(&made.module).map_err(ComputeError::Shader)?;
            checked.iteration_cap = Some(made.cap);
            work = made.work;
            source = written.into();
        }
        if checked.invocations > self.limits.invocations {
            return Err(ComputeError::Limits(format!(
                "{} invocations a run: a frame's passes may run {} (`World::set_compute_budget`)",
                checked.invocations, self.limits.invocations
            )));
        }
        self.added += 1;
        let pass = Pass {
            added: self.added,
            point: compute.point,
            run: compute.run,
            checked,
            source,
            enabled: true,
            demanded: compute.run == Run::OnDemand,
            due: 0.,
            gpu: None,
            failed: None,
            owner: compute.sandboxed,
            work,
            waited: false,
            chosen: false,
            over: 0,
            over_budget: None,
        };
        let index = match self.passes.iter().position(|(_, p)| p.is_none()) {
            Some(index) => index,
            None => {
                self.passes.push((0, None));
                self.passes.len() - 1
            }
        };
        let point = pass.point;
        let slot = &mut self.passes[index];
        slot.0 = slot.0.wrapping_add(1);
        slot.1 = Some(pass);
        let id = ComputeId {
            index: index as u32,
            generation: slot.0,
        };
        if !self.reorder(point) {
            self.passes[index].1 = None;
            self.reorder(point);
            return Err(ComputeError::Cycle);
        }
        Ok(id)
    }
    /// `point`'s passes put in order: whether they could be.
    fn reorder(&mut self, point: Point) -> bool {
        let at: Vec<u32> = (0..self.passes.len() as u32)
            .filter(|&i| {
                self.passes[i as usize]
                    .1
                    .as_ref()
                    .is_some_and(|p| p.point == point)
            })
            .collect();
        let list: Vec<(u64, &[(Key, Access)])> = at
            .iter()
            .map(|&i| {
                let p = self.passes[i as usize].1.as_ref().unwrap();
                (p.added, &p.checked.uses[..])
            })
            .collect();
        match order(&list) {
            Some(ordered) => {
                self.order[point.index()] = ordered.into_iter().map(|k| at[k]).collect();
                true
            }
            None => false,
        }
    }
    fn pass_mut(&mut self, id: ComputeId) -> Option<&mut Pass> {
        match self.passes.get_mut(id.index as usize) {
            Some((generation, Some(pass))) if *generation == id.generation => Some(pass),
            _ => None,
        }
    }
    pub(crate) fn remove_compute(&mut self, id: ComputeId) -> bool {
        let Some(point) = self.pass_mut(id).map(|p| p.point) else {
            return false;
        };
        self.passes[id.index as usize].1 = None;
        self.reorder(point);
        true
    }
    pub(crate) fn set_enabled(&mut self, id: ComputeId, on: bool) -> bool {
        self.pass_mut(id)
            .is_some_and(|p| std::mem::replace(&mut p.enabled, on) != on)
    }
    pub(crate) fn demand(&mut self, id: ComputeId) -> bool {
        match self.pass_mut(id) {
            Some(p) if p.run == Run::OnDemand && p.failed.is_none() => {
                p.demanded = true;
                true
            }
            _ => false,
        }
    }
    pub(crate) fn status(&self, id: ComputeId) -> Option<ComputeStatus> {
        let pass = match self.passes.get(id.index as usize) {
            Some((generation, Some(pass))) if *generation == id.generation => pass,
            _ => return None,
        };
        if let Some(gpu_ms) = pass.over_budget {
            return Some(ComputeStatus::OverBudget { gpu_ms });
        }
        Some(match (&pass.failed, &pass.gpu) {
            (Some(why), _) => ComputeStatus::Failed(why.clone()),
            (None, Some((_, Gpu::Ready(..)))) => ComputeStatus::Ready,
            _ => ComputeStatus::Compiling,
        })
    }
    /// Whether it keeps the world drawing: a pass due by the next frame, or
    /// a read on its way.
    pub(crate) fn wants_frames(&self) -> bool {
        self.passes
            .iter()
            .any(|(_, p)| p.as_ref().is_some_and(|p| p.wants_frames(self.clock)))
            || !self.link.reads.borrow().is_empty()
    }
}

/// Device generations: each painter's device gets a number of its own, so
/// a resource made on another is made again.
static GENERATIONS: AtomicU64 = AtomicU64::new(1);

/// The most bytes of WGSL a sandboxed pass's shader may have: parsed on the
/// app's thread when added, so bounded.
const MOST_SANDBOXED_SOURCE: usize = 1 << 20;
/// The most compute passes a world holds: each holds a pipeline.
const MOST_PASSES: usize = 1024;

/// Sandboxed passes a world holds, and those of one owner.
const MOST_SANDBOXED: usize = 64;
const MOST_AN_OWNERS: usize = 16;
/// Invocations a sandboxed pass runs at most: with 64 loop iterations each
/// the least it's left, within its work (`shader_sandbox::Limits`).
const MOST_SANDBOXED_INVOCATIONS: u64 = 1 << 16;
/// Sandboxed work a frame runs, all owners', and one owner's: four and two
/// passes' worth of `shader_sandbox::Limits::work`.
const SANDBOX_FRAME_WORK: u64 = 1 << 27;
const SANDBOX_OWNER_WORK: u64 = 1 << 26;

/// What `frame` holds in a pass's shader.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FrameUniforms {
    time: f32,
    delta: f32,
    index: u32,
    _pad: u32,
}

/// A painter's side of its world's GPU data: the device it's on, and the
/// frame's time.
#[derive(Default)]
pub(crate) struct Frames {
    generation: u64,
    compute: bool,
    uniforms: Option<wgpu::Buffer>,
    sampler: Option<wgpu::Sampler>,
    start: Option<Instant>,
    last: Option<Instant>,
    index: u64,
    time: f64,
    /// Staging buffers of reads done, mapped no more, for the next to take.
    spare: Vec<wgpu::Buffer>,
    /// Passes' GPU time, where the device has timestamps.
    timing: Option<Timing>,
    /// Invocations this frame's passes have run.
    spent: u64,
}

/// Each pass's GPU time, measured between timestamps written as its own
/// compute pass begins and ends, and read back as reads are.
struct Timing {
    set: wgpu::QuerySet,
    /// Timestamps the set holds: two a pass.
    capacity: u32,
    resolve: wgpu::Buffer,
    /// Nanoseconds a tick.
    period: f32,
    /// This frame's timed passes: their index and when they were added.
    timed: Vec<(u32, u64)>,
    /// Frames' timestamps on their way back.
    jobs: Vec<TimingJob>,
}
/// A frame's timestamps on their way back: its timed passes (index, when
/// added), the staging buffer, where it is.
type TimingJob = (Vec<(u32, u64)>, wgpu::Buffer, Stage);
impl Timing {
    /// Frames' timestamps on their way at once, at most: past it, a frame
    /// isn't timed.
    const IN_FLIGHT: usize = 4;
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, capacity: u32) -> Self {
        Self {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("3D compute timing"),
                ty: wgpu::QueryType::Timestamp,
                count: capacity,
            }),
            capacity,
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("3D compute timing"),
                size: u64::from(capacity) * 8,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            period: queue.get_timestamp_period(),
            timed: Vec::new(),
            jobs: Vec::new(),
        }
    }
}

impl Frames {
    pub(crate) fn device_changed(&mut self) {
        self.generation = 0;
        self.uniforms = None;
        self.sampler = None;
        self.spare.clear();
        self.timing = None;
    }

    /// Which of the frame's due passes run, within its invocations: those
    /// left waiting last frame given room first, the rest in their order;
    /// those left out wait for the next. They run in their order either
    /// way.
    fn choose(&mut self, data: &mut Data) {
        let (time, generation) = (self.time, self.generation);
        let limit = data.limits.invocations;
        let order: Vec<u32> = [Point::BeforeFrame, Point::AfterFrame]
            .iter()
            .flat_map(|point| data.order[point.index()].iter().copied())
            .collect();
        let mut due = Vec::new();
        for &at in &order {
            let Some(pass) = data.passes[at as usize].1.as_mut() else {
                continue;
            };
            pass.chosen = false;
            let ready = matches!(&pass.gpu, Some((g, Gpu::Ready(..))) if *g == generation);
            let is_due = pass.enabled
                && pass.failed.is_none()
                && ready
                && match pass.run {
                    Run::EveryFrame => true,
                    Run::OnDemand => pass.demanded,
                    Run::AtRate(_) => time >= pass.due,
                };
            if is_due {
                due.push(at);
            }
        }
        self.spent = 0;
        // Sandboxed work: a frame's, and each owner's.
        let mut sandboxed = 0u64;
        let mut owners: Vec<(Arc<str>, u64)> = Vec::new();
        for waited in [true, false] {
            for &at in &due {
                let pass = data.passes[at as usize].1.as_mut().unwrap();
                if pass.waited != waited {
                    continue;
                }
                let invocations = pass.checked.invocations;
                if self.spent.saturating_add(invocations) > limit {
                    continue;
                }
                if let Some(owner) = &pass.owner {
                    let held = owners
                        .iter()
                        .find(|(o, _)| o == owner)
                        .map_or(0, |(_, w)| *w);
                    if sandboxed.saturating_add(pass.work) > SANDBOX_FRAME_WORK
                        || held.saturating_add(pass.work) > SANDBOX_OWNER_WORK
                    {
                        continue;
                    }
                    sandboxed += pass.work;
                    match owners.iter_mut().find(|(o, _)| o == owner) {
                        Some((_, w)) => *w += pass.work,
                        None => owners.push((owner.clone(), pass.work)),
                    }
                }
                self.spent += invocations;
                pass.chosen = true;
            }
        }
        for &at in &due {
            let pass = data.passes[at as usize].1.as_mut().unwrap();
            pass.waited = !pass.chosen;
        }
    }

    /// Timestamps that came back read into their passes' times: a pass over
    /// its budget so many frames running is turned off.
    fn read_timing(&mut self, data: &mut Data, device: &wgpu::Device) {
        let Some(timing) = &mut self.timing else {
            return;
        };
        let index = self.index;
        let mut mapping = false;
        for (_, staging, stage) in timing.jobs.iter_mut() {
            if let Stage::Copied(at) = *stage
                && at < index
            {
                let flag = Arc::new(AtomicU8::new(0));
                let set = flag.clone();
                staging.map_async(wgpu::MapMode::Read, .., move |result| {
                    set.store(if result.is_ok() { 1 } else { 2 }, Ordering::Release)
                });
                *stage = Stage::Mapping(flag);
            }
            mapping |= matches!(stage, Stage::Mapping(_));
        }
        if mapping {
            let _ = device.poll(wgpu::PollType::Poll);
        }
        let (period, limits) = (timing.period, data.limits);
        let passes = &mut data.passes;
        let histories = &mut data.owners;
        timing.jobs.retain(|(timed, staging, stage)| {
            let Stage::Mapping(flag) = stage else {
                return true;
            };
            match flag.load(Ordering::Acquire) {
                0 => return true,
                1 => {
                    {
                        let mapped = staging.slice(..).get_mapped_range();
                        let ticks: &[u64] = bytemuck::cast_slice(&mapped[..timed.len() * 16]);
                        // A pass is charged the time past the ends of those
                        // before it: a GPU runs passes that share nothing at
                        // once, and one's span then holds another's work.
                        let mut ended = 0u64;
                        let mut owners: Vec<(Arc<str>, f32)> = Vec::new();
                        for (k, &(at, added)) in timed.iter().enumerate() {
                            let (began, end) = (ticks[2 * k], ticks[2 * k + 1]);
                            let own = end.saturating_sub(began.max(ended));
                            ended = ended.max(end);
                            let Some((_, Some(pass))) = passes.get_mut(at as usize) else {
                                continue;
                            };
                            if pass.added != added || pass.failed.is_some() {
                                continue;
                            }
                            let ms = own as f32 * period / 1e6;
                            if let Some(owner) = &pass.owner {
                                match owners.iter_mut().find(|(o, _)| o == owner) {
                                    Some((_, sum)) => *sum += ms,
                                    None => owners.push((owner.clone(), ms)),
                                }
                            }
                            let budget = if pass.owner.is_some() {
                                limits.sandboxed_gpu_ms
                            } else {
                                limits.gpu_ms
                            };
                            let (history, tripped) = judge(pass.over, ms > budget, limits.frames);
                            pass.over = history;
                            if tripped {
                                pass.over_budget = Some(ms);
                                fail(
                                    pass,
                                    format!(
                                        "it took {ms:.2} ms of the GPU, over its {budget} ms, in {} of its last 8 timed frames",
                                        history.count_ones()
                                    ),
                                );
                            }
                        }
                        // An owner's passes together: over its budget as often,
                        // all are turned off.
                        for (owner, ms) in owners {
                            let at = match histories.iter().position(|(o, _)| *o == owner) {
                                Some(at) => at,
                                None => {
                                    histories.push((owner.clone(), 0));
                                    histories.len() - 1
                                }
                            };
                            let (history, tripped) =
                                judge(histories[at].1, ms > limits.owner_gpu_ms, limits.frames);
                            histories[at].1 = history;
                            if !tripped {
                                continue;
                            }
                            for (_, pass) in passes.iter_mut() {
                                if let Some(pass) = pass
                                    && pass.owner.as_ref() == Some(&owner)
                                    && pass.failed.is_none()
                                {
                                    pass.over_budget = Some(ms);
                                    fail(
                                        pass,
                                        format!(
                                            "`{owner}`'s passes took {ms:.2} ms of the GPU together, over its {} ms",
                                            limits.owner_gpu_ms
                                        ),
                                    );
                                }
                            }
                        }
                    }
                    staging.unmap();
                }
                _ => {}
            }
            false
        });
    }

    /// Before the renderer draws: the app's resources made and written,
    /// reads finished, and the passes before the frame recorded. Whether the
    /// world has GPU data at all (`after` is called only if so).
    #[inline]
    pub(crate) fn before(
        &mut self,
        world: &Weak<RefCell<Inner>>,
        frame: &mut ViewportFrame<'_>,
    ) -> bool {
        let Some(world) = world.upgrade() else {
            return false;
        };
        let Some(mut data) = world.borrow_mut().data.take() else {
            return false;
        };
        self.prepare(&mut data, frame);
        world.borrow_mut().data = Some(data);
        true
    }

    #[cold]
    #[inline(never)]
    fn prepare(&mut self, data: &mut Data, frame: &mut ViewportFrame<'_>) {
        let (device, queue) = (frame.device, frame.queue);
        if self.generation == 0 {
            self.generation = GENERATIONS.fetch_add(1, Ordering::Relaxed);
            self.compute = crate::Caps::of(device).compute;
            self.timing = device
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY)
                .then(|| Timing::new(device, queue, 64));
        }
        self.spent = 0;
        let generation = self.generation;
        let start = *self.start.get_or_insert(frame.time);
        let delta = self.last.map_or(0., |last| {
            frame.time.saturating_duration_since(last).as_secs_f32()
        });
        self.last = Some(frame.time);
        self.time = frame.time.saturating_duration_since(start).as_secs_f64();
        // A display's step, if frames come at one.
        data.clock = (self.time, f64::from(delta).clamp(1. / 120., 1. / 30.));
        self.index += 1;

        data.sweep();
        for buffer in data.buffers.iter().filter_map(Weak::upgrade) {
            let made = buffer
                .gpu
                .borrow()
                .as_ref()
                .is_some_and(|(g, _)| *g == generation);
            if made || buffer.refused.get() == generation {
                continue;
            }
            let usage = wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::UNIFORM
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::INDIRECT;
            let contents = buffer.contents.borrow_mut().take();
            // Checked against the device's limits when added; a refusal
            // all the same is the resource's, not the device's handler's.
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let made = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("3D app buffer"),
                size: buffer.size,
                usage,
                mapped_at_creation: contents.is_some(),
            });
            if let Some(error) = now(scope.pop()) {
                eprintln!(
                    "zgx-3d: a buffer of {} bytes couldn't be made: {error}",
                    buffer.size
                );
                buffer.refused.set(generation);
                continue;
            }
            if let Some(contents) = contents {
                made.slice(..)
                    .get_mapped_range_mut()
                    .copy_from_slice(&contents);
                made.unmap();
            }
            *buffer.gpu.borrow_mut() = Some((generation, made));
        }
        for texture in data.textures.iter().filter_map(Weak::upgrade) {
            let made = texture
                .gpu
                .borrow()
                .as_ref()
                .is_some_and(|(g, ..)| *g == generation);
            if made || texture.refused.get() == generation {
                continue;
            }
            let (width, height) = texture.desc.size;
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            // Renderable, so wgpu clears it by a render pass, not a blit.
            let made = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("3D app texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: texture.desc.format.wgpu(),
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = made.create_view(&Default::default());
            if let Some(error) = now(scope.pop()) {
                eprintln!("zgx-3d: a texture of {width} by {height} couldn't be made: {error}");
                texture.refused.set(generation);
                continue;
            }
            *texture.gpu.borrow_mut() = Some((generation, made, view));
        }
        // Writes since the last frame, ahead of this frame's commands.
        let written: Vec<_> = data.link.writes.borrow_mut().drain(..).collect();
        for buffer in written.iter().filter_map(Weak::upgrade) {
            let gpu = buffer.gpu.borrow();
            let Some((_, gpu)) = gpu.as_ref() else {
                continue;
            };
            for (offset, bytes) in buffer.writes.borrow_mut().drain(..) {
                queue.write_buffer(gpu, offset, &bytes);
            }
        }
        self.finish_reads(data, device);
        // A world holding data and no passes (or only particle pools under
        // its cap) runs nothing.
        if data.passes.iter().all(|(_, p)| p.is_none()) {
            return;
        }
        if self.uniforms.is_none() {
            self.uniforms = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("3D compute frame"),
                size: std::mem::size_of::<FrameUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.sampler = Some(device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("3D compute sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }));
        }
        let uniforms = FrameUniforms {
            time: self.time as f32,
            delta,
            index: self.index as u32,
            _pad: 0,
        };
        queue.write_buffer(
            self.uniforms.as_ref().unwrap(),
            0,
            bytemuck::bytes_of(&uniforms),
        );
        self.read_timing(data, device);
        if let Some(timing) = &mut self.timing {
            timing.timed.clear();
            let passes = data.passes.iter().filter(|(_, p)| p.is_some()).count() as u32;
            if passes * 2 > timing.capacity {
                *timing = Timing::new(device, queue, (passes * 2).next_power_of_two());
            }
        }
        self.compile(data, device);
        self.choose(data);
        self.run(data, Point::BeforeFrame, frame.encoder);
    }

    /// After the renderer drew: the passes after the frame recorded, reads
    /// copied within the budget, and whether the world keeps drawing.
    #[cold]
    #[inline(never)]
    pub(crate) fn after(&mut self, world: &Weak<RefCell<Inner>>, frame: &mut ViewportFrame<'_>) {
        let Some(world) = world.upgrade() else {
            return;
        };
        let Some(mut data) = world.borrow_mut().data.take() else {
            return;
        };
        self.run(&mut data, Point::AfterFrame, frame.encoder);
        if let Some(timing) = &mut self.timing
            && !timing.timed.is_empty()
        {
            let n = timing.timed.len() as u32;
            frame
                .encoder
                .resolve_query_set(&timing.set, 0..n * 2, &timing.resolve, 0);
            let staging = frame.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("3D compute timing"),
                size: u64::from(n) * 16,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            frame
                .encoder
                .copy_buffer_to_buffer(&timing.resolve, 0, &staging, 0, u64::from(n) * 16);
            let timed = std::mem::take(&mut timing.timed);
            timing
                .jobs
                .push((timed, staging, Stage::Copied(self.index)));
        }
        self.copy_reads(&mut data, frame);
        let wants = data.wants_frames();
        let mut inner = world.borrow_mut();
        inner.data = Some(data);
        inner.unsettle(UNSETTLED, wants);
    }

    /// Each pass's pipeline for this device: compiling, or made ready.
    fn compile(&mut self, data: &mut Data, device: &wgpu::Device) {
        let generation = self.generation;
        for (_, pass) in data.passes.iter_mut() {
            let Some(pass) = pass else { continue };
            if pass.failed.is_some() {
                continue;
            }
            if !self.compute {
                fail(pass, "the device runs no compute shaders".into());
                continue;
            }
            if pass.owner.is_some() && self.timing.is_none() {
                fail(pass, ComputeError::Untimed.to_string());
                continue;
            }
            if pass.gpu.as_ref().is_none_or(|(g, _)| *g != generation) {
                let slot: Compiled = Arc::default();
                let (device, source, entry) = (
                    device.clone(),
                    pass.source.clone(),
                    pass.checked.entry.clone(),
                );
                let done = slot.clone();
                let job = move || {
                    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
                    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some("3D compute"),
                        source: wgpu::ShaderSource::Wgsl(source.as_ref().into()),
                    });
                    let pipeline =
                        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: Some("3D compute"),
                            layout: None,
                            module: &module,
                            entry_point: Some(&entry),
                            compilation_options: Default::default(),
                            cache: None,
                        });
                    let result = match now(scope.pop()) {
                        None => Ok(pipeline),
                        Some(error) => Err(error.to_string()),
                    };
                    *done.lock().unwrap_or_else(|p| p.into_inner()) = Some(result);
                };
                // A plugin's on a worker of its own: a driver stuck on its
                // shader holds up nobody else's.
                let spawned = if pass.owner.is_some() {
                    compile_sandboxed(job).map(Some)
                } else {
                    compile_off_the_frame(job).map(|()| None)
                };
                let worker = match spawned {
                    Ok(worker) => worker,
                    Err(error) => {
                        fail(pass, format!("no thread to compile it on: {error}"));
                        continue;
                    }
                };
                pass.gpu = Some((generation, Gpu::Compiling(slot, Instant::now(), worker)));
            }
            let Some((_, Gpu::Compiling(slot, since, worker))) = &pass.gpu else {
                continue;
            };
            let (since, worker) = (*since, *worker);
            let deadline =
                std::time::Duration::from_millis(SANDBOXED_COMPILE_MS.load(Ordering::Relaxed));
            let done = slot.lock().unwrap_or_else(|p| p.into_inner()).take();
            match (done, worker) {
                // Queued on a worker let go since: sent again.
                (None, Some(worker)) if worker != SANDBOXED_WORKER.load(Ordering::Relaxed) => {
                    pass.gpu = None;
                }
                (None, Some(worker)) if since.elapsed() > deadline => {
                    pass.gpu = Some((generation, Gpu::Failed));
                    fail(
                        pass,
                        format!("its shader took over {deadline:?} to compile"),
                    );
                    replace_sandboxed_worker(worker);
                }
                (None, _) => {}
                (done, _) => match done.unwrap() {
                    Err(why) => {
                        pass.gpu = Some((generation, Gpu::Failed));
                        fail(pass, why);
                    }
                    Ok(pipeline) => match self.bind(pass, &pipeline, device) {
                        Ok(binds) => pass.gpu = Some((generation, Gpu::Ready(pipeline, binds))),
                        Err(why) => {
                            pass.gpu = Some((generation, Gpu::Failed));
                            fail(pass, why);
                        }
                    },
                },
            }
        }
    }

    /// `pass`'s bind groups for `pipeline`, its resources as made on this
    /// device.
    fn bind(
        &self,
        pass: &Pass,
        pipeline: &wgpu::ComputePipeline,
        device: &wgpu::Device,
    ) -> Result<Vec<wgpu::BindGroup>, String> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut binds = Vec::with_capacity(pass.checked.groups.len());
        for (group, slots) in pass.checked.groups.iter().enumerate() {
            let buffers: Vec<_> = slots
                .iter()
                .map(|(_, slot)| match slot {
                    Slot::Buffer(b) => b.0.gpu.borrow().as_ref().map(|(_, b)| b.clone()),
                    _ => None,
                })
                .collect();
            let views: Vec<_> = slots
                .iter()
                .map(|(_, slot)| match slot {
                    Slot::Texture(t) => t.0.gpu.borrow().as_ref().map(|(.., v)| v.clone()),
                    _ => None,
                })
                .collect();
            let mut entries = Vec::with_capacity(slots.len());
            for (at, (binding, slot)) in slots.iter().enumerate() {
                let resource = match slot {
                    Slot::Buffer(_) => buffers[at]
                        .as_ref()
                        .ok_or("a buffer wasn't made")?
                        .as_entire_binding(),
                    Slot::Texture(_) => wgpu::BindingResource::TextureView(
                        views[at].as_ref().ok_or("a texture wasn't made")?,
                    ),
                    Slot::Frame => self.uniforms.as_ref().unwrap().as_entire_binding(),
                    Slot::Sampler => wgpu::BindingResource::Sampler(self.sampler.as_ref().unwrap()),
                };
                entries.push(wgpu::BindGroupEntry {
                    binding: *binding,
                    resource,
                });
            }
            binds.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("3D compute"),
                layout: &pipeline.get_bind_group_layout(group as u32),
                entries: &entries,
            }));
        }
        match now(scope.pop()) {
            None => Ok(binds),
            Some(error) => Err(error.to_string()),
        }
    }

    /// The passes at `point` that are due, in their order, in one compute
    /// pass.
    fn run(&mut self, data: &mut Data, point: Point, encoder: &mut wgpu::CommandEncoder) {
        let (time, generation) = (self.time, self.generation);
        let mut pass_encoder: Option<wgpu::ComputePass> = None;
        for &index in &data.order[point.index()] {
            let Some(pass) = data.passes[index as usize].1.as_mut() else {
                continue;
            };
            if !pass.enabled || pass.failed.is_some() {
                continue;
            }
            let Some((g, Gpu::Ready(pipeline, binds))) = &pass.gpu else {
                continue;
            };
            if *g != generation {
                continue;
            }
            let due = match pass.run {
                Run::EveryFrame => true,
                Run::OnDemand => pass.demanded,
                Run::AtRate(_) => time >= pass.due,
            };
            if !due {
                continue;
            }
            // Past the frame's invocations, it waits for the next frame,
            // still due (`choose`).
            if !pass.chosen {
                continue;
            }
            // Timed, a pass is a compute pass of its own between two
            // timestamps; untimed, the point's passes share one.
            let timed = self.timing.as_mut().filter(|t| {
                t.jobs.len() < Timing::IN_FLIGHT && (t.timed.len() as u32) * 2 < t.capacity
            });
            if let Some(timing) = timed {
                let k = timing.timed.len() as u32;
                timing.timed.push((index, pass.added));
                // The last pass ended before this one begins.
                drop(pass_encoder.take());
                pass_encoder = Some(
                    encoder
                        .begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("3D app compute"),
                            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                                query_set: &timing.set,
                                beginning_of_pass_write_index: Some(2 * k),
                                end_of_pass_write_index: Some(2 * k + 1),
                            }),
                        })
                        .forget_lifetime(),
                );
            } else if pass_encoder.is_none() {
                // One compute pass for the point's passes; ended (dropped)
                // before the encoder records anything else.
                pass_encoder = Some(
                    encoder
                        .begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("3D app compute"),
                            timestamp_writes: None,
                        })
                        .forget_lifetime(),
                );
            }
            let compute = pass_encoder.as_mut().unwrap();
            compute.set_pipeline(pipeline);
            for (group, bind) in binds.iter().enumerate() {
                compute.set_bind_group(group as u32, bind, &[]);
            }
            let [x, y, z] = pass.checked.workgroups;
            if x > 0 && y > 0 && z > 0 {
                compute.dispatch_workgroups(x, y, z);
            }
            match pass.run {
                Run::OnDemand => pass.demanded = false,
                Run::AtRate(per_second) => {
                    let period = 1. / f64::from(per_second.max(1e-3));
                    pass.due = if time - pass.due > period {
                        time + period
                    } else {
                        pass.due + period
                    };
                }
                Run::EveryFrame => {}
            }
        }
    }

    /// Reads nobody holds let go; chunks copied in an earlier frame mapped,
    /// those mapped taken out, and reads whose chunks are all back
    /// resolved.
    fn finish_reads(&mut self, data: &mut Data, device: &wgpu::Device) {
        let mut reads = data.link.reads.borrow_mut();
        if reads.is_empty() {
            return;
        }
        let (generation, index) = (self.generation, self.index);
        reads.retain(|job| {
            if job.abandoned() {
                return false;
            }
            if !job.chunks.is_empty() && job.generation != generation {
                job.state.resolve(Err(ReadError::DeviceChanged));
                return false;
            }
            true
        });
        let mut mapping = false;
        for chunk in reads.iter_mut().flat_map(|job| job.chunks.iter_mut()) {
            if let Stage::Copied(at) = chunk.stage
                && at < index
            {
                let flag = Arc::new(AtomicU8::new(0));
                let set = flag.clone();
                chunk
                    .staging
                    .map_async(wgpu::MapMode::Read, .., move |result| {
                        set.store(if result.is_ok() { 1 } else { 2 }, Ordering::Release)
                    });
                chunk.stage = Stage::Mapping(flag);
            }
            mapping |= matches!(chunk.stage, Stage::Mapping(_));
        }
        if mapping {
            let _ = device.poll(wgpu::PollType::Poll);
        }
        let spare = &mut self.spare;
        reads.retain_mut(|job| {
            let rows = match &job.source {
                Source::Buffer(_) => None,
                Source::Texture(weak) => weak.upgrade().map(|t| {
                    let row = u64::from(t.desc.size.0) * u64::from(t.desc.format.texel());
                    (
                        row as usize,
                        padded_row(t.desc.size.0, t.desc.format) as usize,
                    )
                }),
            };
            let (chunks, out) = (&mut job.chunks, &mut job.out);
            let mut failed = false;
            chunks.retain(|chunk| {
                let Stage::Mapping(flag) = &chunk.stage else {
                    return true;
                };
                match flag.load(Ordering::Acquire) {
                    0 => return true,
                    1 => {
                        {
                            let mapped = chunk.staging.slice(..).get_mapped_range();
                            let (at, len) = (chunk.at as usize, chunk.len as usize);
                            match rows {
                                None => out[at..at + len].copy_from_slice(&mapped[..len]),
                                Some((row, padded)) => {
                                    for r in 0..len {
                                        out[(at + r) * row..(at + r + 1) * row]
                                            .copy_from_slice(&mapped[r * padded..r * padded + row]);
                                    }
                                }
                            }
                        }
                        chunk.staging.unmap();
                        if spare.len() < SPARE && chunk.staging.size() <= SPARE_SIZE {
                            spare.push(chunk.staging.clone());
                        }
                    }
                    _ => failed = true,
                }
                false
            });
            if failed {
                job.state.resolve(Err(ReadError::DeviceChanged));
                return false;
            }
            if job.issued == job.total && job.chunks.is_empty() {
                let bytes = job.out[job.trim.clone()].to_vec();
                job.state.resolve(Ok(bytes));
                return false;
            }
            true
        });
    }

    /// Chunks of the reads waiting copied into staging buffers, in the order
    /// asked, up to the budget's bytes this frame (at least 4 bytes or a
    /// texture's row, if the budget is narrower), none larger than
    /// `MAX_CHUNK`.
    fn copy_reads(&mut self, data: &mut Data, frame: &mut ViewportFrame<'_>) {
        let mut reads = data.link.reads.borrow_mut();
        let (generation, index) = (self.generation, self.index);
        let mut left = data.budget;
        let mut copied = false;
        let spare = &mut self.spare;
        reads.retain_mut(|job| {
            if job.abandoned() {
                return false;
            }
            if job.issued == job.total || (left == 0 && copied) {
                return true;
            }
            if job.issued > 0 && job.generation != generation {
                job.state.resolve(Err(ReadError::DeviceChanged));
                return false;
            }
            match &job.source {
                Source::Buffer(weak) => {
                    let Some(buffer) = weak.upgrade() else {
                        job.state.resolve(Err(ReadError::Dropped));
                        return false;
                    };
                    let gpu = buffer.gpu.borrow();
                    let Some((_, gpu)) = gpu.as_ref().filter(|(g, _)| *g == generation) else {
                        return true;
                    };
                    job.generation = generation;
                    if job.out.is_empty() {
                        job.out = vec![0; job.total as usize];
                    }
                    while job.issued < job.total {
                        let mut len = (job.total - job.issued).min(left & !3).min(MAX_CHUNK);
                        if len == 0 {
                            if copied {
                                break;
                            }
                            len = (job.total - job.issued).min(4);
                        }
                        let staging = staging(spare, frame.device, len);
                        frame.encoder.copy_buffer_to_buffer(
                            gpu,
                            job.from + job.issued,
                            &staging,
                            0,
                            len,
                        );
                        job.chunks.push(Chunk {
                            at: job.issued,
                            len,
                            staging,
                            stage: Stage::Copied(index),
                        });
                        job.issued += len;
                        left = left.saturating_sub(len);
                        copied = true;
                        if left == 0 {
                            break;
                        }
                    }
                }
                Source::Texture(weak) => {
                    let Some(texture) = weak.upgrade() else {
                        job.state.resolve(Err(ReadError::Dropped));
                        return false;
                    };
                    let gpu = texture.gpu.borrow();
                    let Some((_, gpu, _)) = gpu.as_ref().filter(|(g, ..)| *g == generation) else {
                        return true;
                    };
                    job.generation = generation;
                    if job.out.is_empty() {
                        job.out = vec![0; job.trim.end];
                    }
                    let (width, _) = texture.desc.size;
                    let padded = padded_row(width, texture.desc.format);
                    while job.issued < job.total {
                        let fit = (left / padded).min(MAX_CHUNK / padded);
                        let mut rows = (job.total - job.issued).min(fit);
                        if rows == 0 {
                            if copied {
                                break;
                            }
                            rows = 1;
                        }
                        let staging = staging(spare, frame.device, rows * padded);
                        frame.encoder.copy_texture_to_buffer(
                            wgpu::TexelCopyTextureInfo {
                                texture: gpu,
                                mip_level: 0,
                                origin: wgpu::Origin3d {
                                    x: 0,
                                    y: job.issued as u32,
                                    z: 0,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::TexelCopyBufferInfo {
                                buffer: &staging,
                                layout: wgpu::TexelCopyBufferLayout {
                                    offset: 0,
                                    bytes_per_row: Some(padded as u32),
                                    rows_per_image: None,
                                },
                            },
                            wgpu::Extent3d {
                                width,
                                height: rows as u32,
                                depth_or_array_layers: 1,
                            },
                        );
                        job.chunks.push(Chunk {
                            at: job.issued,
                            len: rows,
                            staging,
                            stage: Stage::Copied(index),
                        });
                        job.issued += rows;
                        left = left.saturating_sub(rows * padded);
                        copied = true;
                        if left == 0 {
                            break;
                        }
                    }
                }
            }
            true
        });
    }
}

/// The most a chunk of a read stages: so a large read never makes a large
/// staging buffer, whatever the budget.
const MAX_CHUNK: u64 = 4 << 20;

/// Staging buffers kept for reads to come, and the largest kept.
const SPARE: usize = 4;
const SPARE_SIZE: u64 = 1 << 20;

/// A staging buffer of at least `size` bytes: one kept if one fits,
/// otherwise made (a power of two, so it fits reads to come).
fn staging(spare: &mut Vec<wgpu::Buffer>, device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    if let Some(at) = spare
        .iter()
        .position(|b| b.size() >= size && b.size() <= size.max(128) * 2)
    {
        return spare.swap_remove(at);
    }
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("3D readback"),
        size: size.next_power_of_two().max(256),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A texture row's bytes as copied: padded to 256.
fn padded_row(width: u32, format: Format) -> u64 {
    (u64::from(width) * u64::from(format.texel())).next_multiple_of(256)
}

/// `pass` can't run: why, reported once.
fn fail(pass: &mut Pass, why: String) {
    eprintln!("zgx-3d: a compute pass can't run: {why}");
    pass.failed = Some(why);
    pass.demanded = false;
}

/// A pass's history of its last 8 timed frames, `slow` the newest, and
/// whether `frames` of them over budget turn it off.
fn judge(history: u8, slow: bool, frames: u32) -> (u8, bool) {
    let history = history << 1 | u8::from(slow);
    (history, history.count_ones() >= frames.clamp(1, 8))
}

/// Work for the compile worker: pipelines and shader modules made off the
/// frame.
type Job = Box<dyn FnOnce() + Send>;

/// Compile workers started in this process: one, ever.
static WORKERS: AtomicU64 = AtomicU64::new(0);

/// `job` run on the process's one compile worker, started with the first
/// job and parked on its queue while there's none: however many passes and
/// devices, one thread. Its error if the worker can't be started.
pub(crate) fn compile_off_the_frame(job: impl FnOnce() + Send + 'static) -> Result<(), String> {
    static QUEUE: Mutex<Option<std::sync::mpsc::Sender<Job>>> = Mutex::new(None);
    on_worker(&QUEUE, Box::new(job))
}

/// `job` on the worker sandboxed code compiles on: apart from the app's, so
/// a driver taking too long over a plugin's shader holds up no other.
/// The worker's generation, as `job` was given it.
pub(crate) fn compile_sandboxed(job: impl FnOnce() + Send + 'static) -> Result<u64, String> {
    if STALLS.load(Ordering::Relaxed) >= MOST_STALLS {
        return Err(format!(
            "sandboxed shaders aren't compiled: drivers stalled on {MOST_STALLS} of them"
        ));
    }
    let generation = SANDBOXED_WORKER.load(Ordering::Relaxed);
    on_worker(&SANDBOXED_QUEUE, Box::new(job))?;
    Ok(generation)
}

/// The sandboxed worker's queue and generation: a driver stuck on a
/// sandboxed shader past the deadline has its worker let go (the thread
/// left to finish or not), a new one taking the queue from then, and passes
/// waiting on the old sent again.
static SANDBOXED_QUEUE: Mutex<Option<std::sync::mpsc::Sender<Job>>> = Mutex::new(None);
static SANDBOXED_WORKER: AtomicU64 = AtomicU64::new(0);
/// Workers let go so far, and the most: past it, sandboxed shaders aren't
/// compiled at all (stuck threads aren't made without end).
static STALLS: AtomicU64 = AtomicU64::new(0);
const MOST_STALLS: u64 = 4;

/// The sandboxed worker of generation `stuck` let go, if it's still the one.
fn replace_sandboxed_worker(stuck: u64) {
    let mut queue = SANDBOXED_QUEUE.lock().unwrap_or_else(|p| p.into_inner());
    if SANDBOXED_WORKER
        .compare_exchange(stuck, stuck + 1, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        *queue = None;
        STALLS.fetch_add(1, Ordering::Relaxed);
    }
}

/// How long a sandboxed pass's pipeline may take to compile (10 s), in
/// milliseconds: past it, the pass fails and its worker is let go.
static SANDBOXED_COMPILE_MS: AtomicU64 = AtomicU64::new(10_000);

/// For tests: the sandboxed compile deadline, and a job holding the
/// sandboxed worker for `stall`, as a driver stuck on a shader would.
#[doc(hidden)]
pub fn stall_sandboxed_compiles(deadline: std::time::Duration, stall: std::time::Duration) {
    SANDBOXED_COMPILE_MS.store(deadline.as_millis() as u64, Ordering::Relaxed);
    let _ = compile_sandboxed(move || std::thread::sleep(stall));
}

fn on_worker(queue: &Mutex<Option<std::sync::mpsc::Sender<Job>>>, job: Job) -> Result<(), String> {
    let mut queue = queue.lock().unwrap_or_else(|p| p.into_inner());
    if queue.is_none() {
        let (send, receive) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("zgx-3d compile".into())
            .spawn(move || {
                for job in receive {
                    job();
                }
            })
            .map_err(|error| error.to_string())?;
        WORKERS.fetch_add(1, Ordering::Relaxed);
        *queue = Some(send);
    }
    queue
        .as_ref()
        .unwrap()
        .send(job)
        .map_err(|_| "the compile worker stopped".to_owned())
}

/// Compile workers this process has started: one for the app's, one for
/// sandboxed code, at most. For tests.
#[doc(hidden)]
pub fn compile_threads() -> u64 {
    WORKERS.load(Ordering::Relaxed)
}

/// `future`, which resolves at once (wgpu's error scopes on native).
pub(crate) fn now<T>(future: impl std::future::Future<Output = T>) -> T {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::yield_now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pass_slow_on_some_of_its_timed_frames_trips_as_one_slow_on_all() {
        // `timed` frames only reach `judge`: an untimed frame between them
        // neither resets nor adds. Slow on alternate timed frames: the third
        // slow one of the last eight trips it.
        let run = |slow: &[bool]| {
            let mut history = 0;
            slow.iter().position(|&s| {
                let (h, tripped) = judge(history, s, 3);
                history = h;
                tripped
            })
        };
        assert_eq!(run(&[true, true, true]), Some(2));
        assert_eq!(run(&[true, false, true, false, true]), Some(4));
        assert_eq!(
            run(&[true, false, false, true, false, false, true]),
            Some(6)
        );
        // Three slow frames more than eight apart: the window forgets.
        let spread = [true, false, false, false, false, false, false, false, false];
        let three = [&spread[..], &spread[..], &[true][..]].concat();
        assert_eq!(run(&three), None);
        assert_eq!(run(&[false; 20]), None);
    }

    #[test]
    fn the_apps_data_is_on_the_frames_device_before_the_renderer_draws_and_says_which() {
        use zgx_gpu::viewport::Viewport;
        let world = crate::World::new();
        let buffer = world.add_buffer(crate::BufferDesc::bytes(64)).unwrap();
        let texture = world
            .add_texture(crate::TextureDesc::storage(4, 4, Format::Rgba8Unorm))
            .unwrap();
        assert!(buffer.device_buffer().is_none() && texture.device_view().is_none());
        let draw = |painter: &mut dyn Viewport, (device, queue): &(wgpu::Device, wgpu::Queue)| {
            let format = wgpu::TextureFormat::Bgra8Unorm;
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 4,
                    height: 4,
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
            let mut encoder = device.create_command_encoder(&Default::default());
            let mut frame =
                ViewportFrame::new(device, queue, &mut encoder, &view, format, (4, 4), 1.);
            painter.render(&mut frame);
            queue.submit([encoder.finish()]);
        };
        let mut painter = world.viewport();
        let first = wgpu::Device::noop(&Default::default());
        draw(&mut painter, &first);
        let (generation, made) = buffer.device_buffer().expect("made with the frame");
        assert_eq!(made.size(), 64);
        assert_eq!(texture.device_view().unwrap().0, generation);
        // A new device: made again, under another generation.
        painter.device_changed();
        let second = wgpu::Device::noop(&Default::default());
        draw(&mut painter, &second);
        let (again, _) = buffer.device_buffer().unwrap();
        assert_ne!(again, generation);
        assert_eq!(texture.device_view().unwrap().0, again);
    }

    #[test]
    fn a_whole_write_goes_before_earlier_readers_and_the_rest_keep_their_order() {
        let (x, y) = (Key::Buffer(1), Key::Buffer(2));
        use Access::*;
        // Added: 0 reads x; 1 writes x whole. 1 runs first.
        assert_eq!(
            order(&[(1, &[(x, Read)]), (2, &[(x, Write)])]),
            Some(vec![1, 0])
        );
        // An update in place keeps the order added.
        assert_eq!(
            order(&[(1, &[(x, Read)]), (2, &[(x, ReadWrite)])]),
            Some(vec![0, 1])
        );
        assert_eq!(
            order(&[(1, &[(x, ReadWrite)]), (2, &[(x, ReadWrite)])]),
            Some(vec![0, 1])
        );
        // Readers alone: as added.
        assert_eq!(
            order(&[(2, &[(x, Read)]), (1, &[(x, Read)])]),
            Some(vec![1, 0])
        );
        // 0 reads x and writes y; 1 reads y and writes x whole: a cycle.
        assert_eq!(
            order(&[(1, &[(x, Read), (y, Write)]), (2, &[(y, Read), (x, Write)])]),
            None
        );
    }
}
