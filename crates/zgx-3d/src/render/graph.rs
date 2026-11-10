//! The frame as a compiled list of passes (design-3d-aaa.md, 4.1). Each
//! pass says which resources it reads and writes, by name; the list is put
//! in their order, and its temporary textures taken from a pool
//! (`targets`), when the world's configuration or the frame's size changes,
//! never a frame. A frame walks the list. With nothing turned on it is the
//! renderer's three passes: the sun's shadows (with shadows on), what is
//! behind meshes light passes through (with any), and the meshes, drawn
//! straight into the view's target.
use super::{
    Behind, DEPTH,
    targets::{Desc, Pool, Temporary, assign},
};
use zgx_gpu::wgpu;

/// What passes read and write, by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Resource {
    /// The sun's depth: kept between frames, and drawn again only when what
    /// it holds changed (`Gpu::shadow_drawn`).
    ShadowMap,
    /// What is behind meshes light passes through, every level.
    Behind,
    /// The multisampled colour meshes draw into, resolved into the target
    /// (none drawing one sample a pixel: they draw into the target).
    Colour,
    /// The depth meshes test against.
    Depth,
    /// The view's own texture.
    Target,
}
impl Resource {
    /// Whether the list's textures hold it, a frame's alone: the shadow map
    /// is kept, the target the view's.
    fn temporary(self) -> bool {
        matches!(self, Self::Behind | Self::Colour | Self::Depth)
    }
}

/// A pass of the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pass {
    /// Opaque meshes' depth as the sun sees them.
    Shadows,
    /// The meshes but those light passes through, into what is behind them.
    Behind,
    /// The meshes, into the target.
    Main,
}
impl Pass {
    /// How many there are: the most a list holds.
    const COUNT: usize = 3;
    pub(super) fn reads(self) -> &'static [Resource] {
        match self {
            Self::Shadows => &[],
            Self::Behind => &[Resource::ShadowMap],
            Self::Main => &[Resource::ShadowMap, Resource::Behind],
        }
    }
    pub(super) fn writes(self) -> &'static [Resource] {
        match self {
            Self::Shadows => &[Resource::ShadowMap],
            Self::Behind => &[Resource::Colour, Resource::Depth, Resource::Behind],
            Self::Main => &[Resource::Colour, Resource::Depth, Resource::Target],
        }
    }
}

/// Passes in the order a frame runs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct List {
    passes: [Pass; Pass::COUNT],
    len: u8,
}
impl List {
    pub(super) fn iter(&self) -> impl Iterator<Item = Pass> + '_ {
        self.passes[..usize::from(self.len)].iter().copied()
    }
}

/// `passes` (each at most once) in an order where each runs after those
/// writing what it reads; passes otherwise apart keep the order given.
/// `None` if they can't be: some read what each other writes.
pub(super) fn order(passes: &[Pass]) -> Option<List> {
    let before = |a: Pass, b: Pass| a != b && a.writes().iter().any(|r| b.reads().contains(r));
    let mut list = List {
        passes: [Pass::Main; Pass::COUNT],
        len: 0,
    };
    let mut placed = [false; Pass::COUNT];
    while usize::from(list.len) < passes.len() {
        // The first not placed whose writers are all placed.
        let next = (0..passes.len()).find(|&i| {
            !placed[i] && (0..passes.len()).all(|j| placed[j] || !before(passes[j], passes[i]))
        })?;
        placed[next] = true;
        list.passes[usize::from(list.len)] = passes[next];
        list.len += 1;
    }
    Some(list)
}

/// What the list follows: when any of it changes, it's compiled again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Config {
    /// The frame's size, in pixels.
    pub(super) size: (u32, u32),
    /// The sun casts shadows (`World::set_shadows`).
    pub(super) shadows: bool,
    /// Light passes through some mesh (`World::set_transmission`).
    pub(super) transmits: bool,
}

/// The multisampled colour and the depth meshes draw into.
pub(super) struct Targets {
    pub(super) colour: Option<wgpu::TextureView>,
    pub(super) depth: wgpu::TextureView,
    textures: Vec<Temporary>,
}

/// The frame's compiled list and its temporaries, made with the device's
/// renderer (`Gpu`): it draws into `format` at `samples` a pixel.
pub(super) struct Graph {
    format: wgpu::TextureFormat,
    samples: u32,
    /// What it was compiled for; `None` before the first frame and after
    /// `rest`.
    config: Option<Config>,
    passes: List,
    pub(super) targets: Option<Targets>,
    /// What is behind meshes light passes through, while any does.
    pub(super) behind: Option<Behind>,
    pool: Pool,
}
impl Graph {
    pub(super) fn new(format: wgpu::TextureFormat, samples: u32) -> Self {
        Self {
            format,
            samples,
            config: None,
            passes: List {
                passes: [Pass::Main; Pass::COUNT],
                len: 0,
            },
            targets: None,
            behind: None,
            pool: Pool::default(),
        }
    }
    /// The list for `config`, compiled if it was for another: whether what
    /// is behind was made again too (bindings reading the old must go).
    #[inline]
    pub(super) fn prepare(&mut self, device: &wgpu::Device, config: Config) -> bool {
        if self.config == Some(config) {
            return false;
        }
        self.compile(device, config)
    }
    /// The passes, in the order a frame runs them.
    pub(super) fn passes(&self) -> List {
        self.passes
    }
    /// Lets go of every temporary (`Viewport::rest`, the host's idle trim):
    /// the next frame compiles the list again.
    pub(super) fn rest(&mut self) {
        self.config = None;
        self.targets = None;
        self.behind = None;
        self.pool.clear();
    }
    // (`World::memory` will read it: design-3d-aaa.md, 4.3.)
    #[cfg(test)]
    /// The temporaries' bytes on the device.
    pub(super) fn bytes(&self) -> u64 {
        let targets = self.targets.iter().flat_map(|t| &t.textures);
        let behind = self.behind.iter().map(|b| &b.texture);
        targets.chain(behind).map(|t| t.desc.bytes()).sum()
    }
    /// The list for `config` and its temporaries: those it had given back to
    /// the pool and taken again where alike, what is behind kept if its size
    /// is.
    #[cold]
    #[inline(never)]
    fn compile(&mut self, device: &wgpu::Device, config: Config) -> bool {
        let wanted = [
            config.shadows.then_some(Pass::Shadows),
            config.transmits.then_some(Pass::Behind),
            Some(Pass::Main),
        ];
        let wanted: Vec<Pass> = wanted.into_iter().flatten().collect();
        self.passes = order(&wanted).expect("the stock passes have an order");
        // Each temporary the list uses, by the passes first and last using
        // it: what is behind apart (its levels are its own).
        let target = |size, format| Desc {
            size,
            format,
            samples: self.samples,
            levels: 1,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        };
        let mut used: Vec<(Resource, Desc, usize, usize)> = Vec::new();
        for (at, pass) in self.passes.iter().enumerate() {
            for &resource in pass.reads().iter().chain(pass.writes()) {
                let desc = match resource {
                    Resource::Colour if self.samples > 1 => target(config.size, self.format),
                    Resource::Depth => target(config.size, DEPTH),
                    _ => continue,
                };
                debug_assert!(resource.temporary());
                match used.iter_mut().find(|(r, ..)| *r == resource) {
                    Some((.., last)) => *last = at,
                    None => used.push((resource, desc, at, at)),
                }
            }
        }
        if let Some(targets) = self.targets.take() {
            targets.textures.into_iter().for_each(|t| self.pool.give(t));
        }
        let lifetimes: Vec<(Desc, usize, usize)> = used
            .iter()
            .map(|&(_, d, first, last)| (d, first, last))
            .collect();
        let (slots, count) = assign(&lifetimes);
        let mut textures: Vec<Option<Temporary>> = (0..count).map(|_| None).collect();
        for (&slot, &(resource, desc, ..)) in slots.iter().zip(&used) {
            let label = match resource {
                Resource::Colour => "3D colour",
                _ => "3D depth",
            };
            textures[slot].get_or_insert_with(|| self.pool.take(device, desc, label));
        }
        let view = |resource| {
            let at = used.iter().position(|(r, ..)| *r == resource)?;
            Some(
                textures[slots[at]]
                    .as_ref()?
                    .texture
                    .create_view(&Default::default()),
            )
        };
        let (colour, depth) = (view(Resource::Colour), view(Resource::Depth));
        self.targets = Some(Targets {
            colour,
            depth: depth.expect("the meshes' pass tests depth"),
            textures: textures.into_iter().flatten().collect(),
        });
        // What is behind, at the frame's size, while light passes through.
        let behind_made = match &self.behind {
            _ if !config.transmits => {
                self.behind = None;
                false
            }
            Some(behind) if behind.size == config.size => false,
            _ => {
                if let Some(old) = self.behind.take() {
                    self.pool.give(old.texture);
                }
                let desc = Behind::desc(self.format, config.size);
                let texture = self.pool.take(device, desc, "3D behind");
                self.behind = Some(Behind::new(device, self.format, config.size, texture));
                true
            }
        };
        self.pool.clear();
        self.config = Some(config);
        behind_made
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pass_runs_after_those_writing_what_it_reads() {
        let list = |passes: &[Pass]| order(passes).map(|l| l.iter().collect::<Vec<_>>());
        let all = vec![Pass::Shadows, Pass::Behind, Pass::Main];
        assert_eq!(
            list(&[Pass::Main, Pass::Behind, Pass::Shadows]),
            Some(all.clone())
        );
        assert_eq!(list(&[Pass::Behind, Pass::Main, Pass::Shadows]), Some(all));
        assert_eq!(list(&[Pass::Main]), Some(vec![Pass::Main]));
        // Apart from each other, as given.
        assert_eq!(
            list(&[Pass::Main, Pass::Shadows]),
            Some(vec![Pass::Shadows, Pass::Main])
        );
    }

    /// `future`, of wgpu's no-op backend, which never waits.
    fn now<T>(future: impl std::future::Future<Output = T>) -> T {
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match std::pin::pin!(future).poll(&mut context) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("the no-op backend waited"),
        }
    }

    /// A device of wgpu's no-op backend: textures made, none drawn.
    fn device() -> wgpu::Device {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::NOOP;
        descriptor.backend_options.noop.enable = true;
        let instance = wgpu::Instance::new(descriptor);
        let adapter = now(instance.request_adapter(&Default::default()));
        let adapter = adapter.expect("the no-op adapter");
        now(adapter.request_device(&Default::default()))
            .expect("the no-op device")
            .0
    }

    #[test]
    fn a_list_is_compiled_when_its_configuration_changes_and_rests_holding_nothing() {
        let device = device();
        let mut graph = Graph::new(wgpu::TextureFormat::Bgra8UnormSrgb, 4);
        let plain = Config {
            size: (64, 32),
            shadows: false,
            transmits: false,
        };
        assert!(!graph.prepare(&device, plain));
        assert_eq!(graph.passes().iter().collect::<Vec<_>>(), [Pass::Main]);
        // Colour and depth, four samples a pixel.
        let targets = 64 * 32 * 4 * (4 + 4);
        assert_eq!(graph.bytes(), targets);
        let depth = graph.targets.as_ref().unwrap().depth.clone();
        // The same again keeps its textures; another size makes them again.
        assert!(!graph.prepare(&device, plain));
        assert!(graph.targets.as_ref().unwrap().depth == depth);
        let glass = Config {
            transmits: true,
            shadows: true,
            ..plain
        };
        assert!(graph.prepare(&device, glass), "what is behind made");
        assert_eq!(
            graph.passes().iter().collect::<Vec<_>>(),
            [Pass::Shadows, Pass::Behind, Pass::Main]
        );
        // What is behind: 64 by 32 and its six levels below.
        let behind: u64 = (0..7)
            .map(|l| (64u64 >> l).max(1) * (32u64 >> l).max(1) * 4)
            .sum();
        assert_eq!(graph.bytes(), targets + behind);
        assert!(
            !graph.prepare(
                &device,
                Config {
                    shadows: false,
                    ..glass
                }
            ),
            "what is behind kept at its size"
        );
        assert!(graph.prepare(
            &device,
            Config {
                size: (32, 32),
                ..glass
            }
        ));
        graph.rest();
        assert_eq!(graph.bytes(), 0);
        assert!(graph.targets.is_none() && graph.behind.is_none());
        assert!(graph.prepare(&device, plain) || graph.targets.is_some());
        assert_eq!(graph.bytes(), targets);
    }
}
