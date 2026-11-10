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
        BufferShared, DEFAULT_MEMORY_CAP, DEFAULT_READBACK_BUDGET, Format, GpuBuffer, GpuTexture,
        Link, ReadError, ReadJob, Source, Stage, TextureShared,
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
    /// once a frame: the world keeps drawing.
    pub fn at_rate(mut self, per_second: f32) -> Self {
        self.run = Run::AtRate(per_second);
        self
    }
    /// Where in the frame it runs.
    pub fn at(mut self, point: Point) -> Self {
        self.point = point;
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
            Self::Cycle => f.write_str(
                "the pass writes what an earlier pass reads, which writes what it reads",
            ),
        }
    }
}
impl std::error::Error for ComputeError {}

/// Where a pass is on the device (`World::compute_status`).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComputeStatus {
    /// Waiting for a frame, or compiling.
    Compiling,
    /// Compiled: it runs when due.
    Ready,
    /// The device refused it, or runs no compute: why.
    Failed(String),
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
    Compiling(Compiled),
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
}
impl Pass {
    fn wants_frames(&self) -> bool {
        self.enabled
            && self.failed.is_none()
            && match self.run {
                Run::EveryFrame | Run::AtRate(_) => true,
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
    pub(crate) cap: u64,
    pub(crate) budget: u64,
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
            cap: DEFAULT_MEMORY_CAP,
            budget: DEFAULT_READBACK_BUDGET,
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
        let used = self.buffer_bytes + self.texture_bytes;
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
    pub(crate) fn add_compute(&mut self, compute: Compute) -> Result<ComputeId, ComputeError> {
        if compute.point == Point::BeforeTransparents {
            return Err(ComputeError::Point(compute.point));
        }
        let checked = check(&compute)?;
        self.added += 1;
        let pass = Pass {
            added: self.added,
            point: compute.point,
            run: compute.run,
            checked,
            source: compute.source,
            enabled: true,
            demanded: compute.run == Run::OnDemand,
            due: 0.,
            gpu: None,
            failed: None,
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
        Some(match (&pass.failed, &pass.gpu) {
            (Some(why), _) => ComputeStatus::Failed(why.clone()),
            (None, Some((_, Gpu::Ready(..)))) => ComputeStatus::Ready,
            _ => ComputeStatus::Compiling,
        })
    }
    /// Whether it keeps the world drawing: a pass due, or a read on its way.
    pub(crate) fn wants_frames(&self) -> bool {
        self.passes
            .iter()
            .any(|(_, p)| p.as_ref().is_some_and(Pass::wants_frames))
            || !self.link.reads.borrow().is_empty()
    }
}

/// Device generations: each painter's device gets a number of its own, so
/// a resource made on another is made again.
static GENERATIONS: AtomicU64 = AtomicU64::new(1);

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
}
impl Frames {
    pub(crate) fn device_changed(&mut self) {
        self.generation = 0;
        self.uniforms = None;
        self.sampler = None;
        self.spare.clear();
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
        }
        let generation = self.generation;
        let start = *self.start.get_or_insert(frame.time);
        let delta = self.last.map_or(0., |last| {
            frame.time.saturating_duration_since(last).as_secs_f32()
        });
        self.last = Some(frame.time);
        self.time = frame.time.saturating_duration_since(start).as_secs_f64();
        self.index += 1;

        data.sweep();
        for buffer in data.buffers.iter().filter_map(Weak::upgrade) {
            let made = buffer
                .gpu
                .borrow()
                .as_ref()
                .is_some_and(|(g, _)| *g == generation);
            if made {
                continue;
            }
            let usage = wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::UNIFORM
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::INDIRECT;
            let contents = buffer.contents.borrow_mut().take();
            let made = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("3D app buffer"),
                size: buffer.size,
                usage,
                mapped_at_creation: contents.is_some(),
            });
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
            if made {
                continue;
            }
            let (width, height) = texture.desc.size;
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
        self.compile(data, device);
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
            if pass.gpu.as_ref().is_none_or(|(g, _)| *g != generation) {
                let slot: Compiled = Arc::default();
                let (device, source, entry) = (
                    device.clone(),
                    pass.source.clone(),
                    pass.checked.entry.clone(),
                );
                let done = slot.clone();
                let spawned = std::thread::Builder::new()
                    .name("zgx-3d compute".into())
                    .spawn(move || {
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
                    });
                if let Err(error) = spawned {
                    fail(pass, format!("no thread to compile it on: {error}"));
                    continue;
                }
                pass.gpu = Some((generation, Gpu::Compiling(slot)));
            }
            let Some((_, Gpu::Compiling(slot))) = &pass.gpu else {
                continue;
            };
            let done = slot.lock().unwrap_or_else(|p| p.into_inner()).take();
            match done {
                None => {}
                Some(Err(why)) => {
                    pass.gpu = Some((generation, Gpu::Failed));
                    fail(pass, why);
                }
                Some(Ok(pipeline)) => match self.bind(pass, &pipeline, device) {
                    Ok(binds) => pass.gpu = Some((generation, Gpu::Ready(pipeline, binds))),
                    Err(why) => {
                        pass.gpu = Some((generation, Gpu::Failed));
                        fail(pass, why);
                    }
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
            if pass_encoder.is_none() {
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

    /// Reads copied whole in an earlier frame mapped, and those mapped
    /// resolved.
    fn finish_reads(&mut self, data: &mut Data, device: &wgpu::Device) {
        let mut reads = data.link.reads.borrow_mut();
        if reads.is_empty() {
            return;
        }
        let (generation, index) = (self.generation, self.index);
        let mut mapping = false;
        for job in reads.iter_mut() {
            if !matches!(job.stage, Stage::Queued) && job.generation != generation {
                job.state.resolve(Err(ReadError::DeviceChanged));
                continue;
            }
            if let Stage::Copied(at) = job.stage
                && at < index
            {
                let flag = Arc::new(AtomicU8::new(0));
                let set = flag.clone();
                job.staging
                    .as_ref()
                    .unwrap()
                    .map_async(wgpu::MapMode::Read, .., move |result| {
                        set.store(if result.is_ok() { 1 } else { 2 }, Ordering::Release)
                    });
                job.stage = Stage::Mapping(flag);
            }
            mapping |= matches!(job.stage, Stage::Mapping(_));
        }
        if mapping {
            let _ = device.poll(wgpu::PollType::Poll);
        }
        let spare = &mut self.spare;
        reads.retain_mut(|job| {
            if job.state.done() {
                return false;
            }
            let Stage::Mapping(flag) = &job.stage else {
                return true;
            };
            match flag.load(Ordering::Acquire) {
                0 => true,
                1 => {
                    let staging = job.staging.take().unwrap();
                    let bytes = extract(job, &staging.slice(..).get_mapped_range());
                    staging.unmap();
                    if spare.len() < SPARE && staging.size() <= SPARE_SIZE {
                        spare.push(staging);
                    }
                    job.state.resolve(Ok(bytes));
                    false
                }
                _ => {
                    job.state.resolve(Err(ReadError::DeviceChanged));
                    false
                }
            }
        });
    }

    /// Reads waiting their turn copied into their staging buffers, in the
    /// order asked, up to the budget's bytes this frame (at least a texture's
    /// row, if the budget is narrower).
    fn copy_reads(&mut self, data: &mut Data, frame: &mut ViewportFrame<'_>) {
        let mut reads = data.link.reads.borrow_mut();
        let (generation, index) = (self.generation, self.index);
        let mut left = data.budget;
        let mut copied = false;
        let spare = &mut self.spare;
        reads.retain_mut(|job| {
            if !matches!(job.stage, Stage::Queued) {
                return true;
            }
            if left == 0 && copied {
                return true;
            }
            if job.copied > 0 && job.generation != generation {
                job.copied = 0;
                job.staging = None;
            }
            match &job.source {
                Source::Buffer(weak) => {
                    let Some(buffer) = weak.upgrade() else {
                        job.state.resolve(Err(ReadError::Dropped));
                        return false;
                    };
                    let gpu = buffer.gpu.borrow();
                    let Some((g, gpu)) = gpu.as_ref().filter(|(g, _)| *g == generation) else {
                        return true;
                    };
                    let staging = job
                        .staging
                        .get_or_insert_with(|| staging(spare, frame.device, job.total));
                    job.generation = *g;
                    let mut chunk = (job.total - job.copied).min(left & !3);
                    if chunk == 0 && !copied {
                        chunk = (job.total - job.copied).min(4);
                    }
                    if chunk > 0 {
                        frame.encoder.copy_buffer_to_buffer(
                            gpu,
                            job.from + job.copied,
                            staging,
                            job.copied,
                            chunk,
                        );
                    }
                    job.copied += chunk;
                    left = left.saturating_sub(chunk);
                    copied |= chunk > 0;
                }
                Source::Texture(weak) => {
                    let Some(texture) = weak.upgrade() else {
                        job.state.resolve(Err(ReadError::Dropped));
                        return false;
                    };
                    let gpu = texture.gpu.borrow();
                    let Some((g, gpu, _)) = gpu.as_ref().filter(|(g, ..)| *g == generation) else {
                        return true;
                    };
                    let (width, _) = texture.desc.size;
                    let padded = padded_row(width, texture.desc.format);
                    let staging = job
                        .staging
                        .get_or_insert_with(|| staging(spare, frame.device, padded * job.total));
                    job.generation = *g;
                    let mut rows = (job.total - job.copied).min(left / padded);
                    if rows == 0 && !copied {
                        rows = 1;
                    }
                    if rows > 0 {
                        frame.encoder.copy_texture_to_buffer(
                            wgpu::TexelCopyTextureInfo {
                                texture: gpu,
                                mip_level: 0,
                                origin: wgpu::Origin3d {
                                    x: 0,
                                    y: job.copied as u32,
                                    z: 0,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::TexelCopyBufferInfo {
                                buffer: staging,
                                layout: wgpu::TexelCopyBufferLayout {
                                    offset: job.copied * padded,
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
                    }
                    job.copied += rows;
                    left = left.saturating_sub(rows * padded);
                    copied |= rows > 0;
                }
            }
            if job.copied == job.total {
                job.stage = Stage::Copied(index);
            }
            true
        });
    }
}

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

/// The bytes `job` asked for, from its staging buffer's `mapped`.
fn extract(job: &ReadJob, mapped: &[u8]) -> Vec<u8> {
    match &job.source {
        Source::Buffer(_) => mapped[job.trim.clone()].to_vec(),
        Source::Texture(weak) => {
            let rows = job.total as usize;
            let row = job.trim.len() / rows.max(1);
            let padded = match weak.upgrade() {
                Some(t) => padded_row(t.desc.size.0, t.desc.format) as usize,
                None => row.next_multiple_of(256),
            };
            let mut out = Vec::with_capacity(row * rows);
            for y in 0..rows {
                out.extend_from_slice(&mapped[y * padded..y * padded + row]);
            }
            out
        }
    }
}

/// `pass` can't run: why, reported once.
fn fail(pass: &mut Pass, why: String) {
    eprintln!("zgx-3d: a compute pass can't run: {why}");
    pass.failed = Some(why);
    pass.demanded = false;
}

/// `future`, which resolves at once (wgpu's error scopes on native).
fn now<T>(future: impl std::future::Future<Output = T>) -> T {
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
