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
    /// The multisampled colour meshes draw into, resolved into the scene
    /// (none drawing one sample a pixel: they draw into it). With the post
    /// chain, kept for a pass of its own to resolve.
    Colour,
    /// The depth meshes test against.
    Depth,
    /// What the meshes drew, one sample a pixel: in linear light into a
    /// float texture of its own where the post chain follows, else the
    /// view's target.
    Scene,
    /// Bloom's steps, each half the size of the one before.
    Bloom,
    /// The scene's other half while the app's HDR passes trade it back and
    /// forth with the scene.
    Scene2,
    /// What the display shows, written by the last pass for the app's
    /// display passes, and its other half.
    Shown,
    Shown2,
    /// The view's own texture (without the post chain, the scene).
    #[cfg_attr(not(feature = "post"), allow(dead_code))]
    Target,
}
impl Resource {
    /// Whether the list's textures hold it, a frame's alone: the shadow map
    /// is kept, the target the view's.
    fn temporary(self) -> bool {
        matches!(
            self,
            Self::Behind
                | Self::Colour
                | Self::Depth
                | Self::Scene
                | Self::Bloom
                | Self::Scene2
                | Self::Shown
                | Self::Shown2
        )
    }
}

/// A pass of the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pass {
    /// Opaque meshes' depth as the sun sees them.
    Shadows,
    /// The meshes but those light passes through, into what is behind them.
    Behind,
    /// The meshes, into the scene.
    Main,
    /// The meshes' samples averaged into the scene, each weighted by how
    /// bright it is so a bright edge is as smooth as a dim one.
    #[cfg(feature = "post")]
    Resolve,
    /// The app's HDR passes (`World::add_post`), in turn.
    #[cfg(feature = "post")]
    AppHdr,
    /// The scene's bloom, its steps made smaller then larger again.
    #[cfg(feature = "post")]
    Bloom,
    /// The scene exposed, its tones mapped, bloom mixed in, into the target.
    #[cfg(feature = "post")]
    Final,
    /// The app's display passes, in turn, the last into the target.
    #[cfg(feature = "post")]
    AppDisplay,
}
impl Pass {
    /// How many there are: the most a list holds.
    const COUNT: usize = if cfg!(feature = "post") { 8 } else { 3 };
    pub(super) fn reads(self) -> &'static [Resource] {
        match self {
            Self::Shadows => &[],
            Self::Behind => &[Resource::ShadowMap],
            Self::Main => &[Resource::ShadowMap, Resource::Behind],
            #[cfg(feature = "post")]
            Self::Resolve => &[Resource::Colour],
            #[cfg(feature = "post")]
            Self::AppHdr => &[Resource::Scene, Resource::Scene2],
            #[cfg(feature = "post")]
            Self::AppDisplay => &[Resource::Shown, Resource::Shown2],
            #[cfg(feature = "post")]
            Self::Bloom => &[Resource::Scene],
            #[cfg(feature = "post")]
            Self::Final => &[Resource::Scene, Resource::Bloom],
        }
    }
    pub(super) fn writes(self) -> &'static [Resource] {
        match self {
            Self::Shadows => &[Resource::ShadowMap],
            Self::Behind => &[Resource::Colour, Resource::Depth, Resource::Behind],
            Self::Main => &[Resource::Colour, Resource::Depth, Resource::Scene],
            #[cfg(feature = "post")]
            Self::Resolve => &[Resource::Scene],
            #[cfg(feature = "post")]
            Self::AppHdr => &[Resource::Scene, Resource::Scene2],
            #[cfg(feature = "post")]
            Self::AppDisplay => &[Resource::Target],
            #[cfg(feature = "post")]
            Self::Bloom => &[Resource::Bloom],
            #[cfg(feature = "post")]
            Self::Final => &[Resource::Target, Resource::Shown],
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
    /// The post chain's passes (`World::set_post`).
    pub(super) post: Chain,
}

/// The post chain's passes on: with none, the meshes draw into the target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Chain {
    /// Any pass is on: the scene is drawn into a float texture, and the
    /// last pass writes the target.
    pub(super) on: bool,
    pub(super) bloom: bool,
    /// The app's passes in the HDR stage and the display stage.
    pub(super) hdr: u8,
    pub(super) display: u8,
}

/// The scene's format where the post chain follows: linear light, past 1.
pub(super) const SCENE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// What bloom's steps are drawn in on `device`: three floats in 4 bytes
/// where it can draw into them, else the scene's format.
pub(super) fn bloom_format(device: &wgpu::Device) -> wgpu::TextureFormat {
    if device
        .features()
        .contains(wgpu::Features::RG11B10UFLOAT_RENDERABLE)
    {
        wgpu::TextureFormat::Rg11b10Ufloat
    } else {
        SCENE
    }
}

/// Bloom's steps for a frame of `size`, in `format`: the first half its
/// size, each half the last, at most 6, the smallest at least 2 pixels a
/// side where the frame allows.
pub(super) fn bloom_desc(size: (u32, u32), format: wgpu::TextureFormat) -> Desc {
    let half = ((size.0 / 2).max(1), (size.1 / 2).max(1));
    let levels = half.0.min(half.1).ilog2().clamp(1, 6);
    Desc {
        size: half,
        format,
        samples: 1,
        levels,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    }
}

/// The multisampled colour and the depth meshes draw into, and the post
/// chain's.
pub(super) struct Targets {
    pub(super) colour: Option<wgpu::TextureView>,
    pub(super) depth: wgpu::TextureView,
    /// The scene, where the post chain follows (else the meshes draw into
    /// the target).
    #[cfg_attr(not(feature = "post"), allow(dead_code))]
    pub(super) scene: Option<wgpu::TextureView>,
    /// The samples are kept for the chain to resolve (`Pass::Resolve`), not
    /// resolved as the meshes' pass ends.
    pub(super) stored: bool,
    /// The scene's other half, what the display shows and its other half,
    /// for the app's passes (`Resource::Scene2`, `Shown`, `Shown2`).
    #[cfg_attr(not(feature = "post"), allow(dead_code))]
    pub(super) others: [Option<wgpu::TextureView>; 3],
    /// Each of bloom's steps, with it on.
    #[cfg_attr(not(feature = "post"), allow(dead_code))]
    pub(super) bloom: Vec<wgpu::TextureView>,
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
    /// What its temporaries hold (`bytes`).
    bytes: u64,
    /// Compiles so far: bindings of its targets are made again as it
    /// changes.
    pub(super) compiled: u64,
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
            bytes: 0,
            compiled: 0,
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
        self.bytes = 0;
    }
    /// The frame's size it was compiled for, in pixels.
    pub(super) fn size(&self) -> (u32, u32) {
        self.config.map_or((0, 0), |c| c.size)
    }
    /// The temporaries' bytes on the device (summed as it compiles).
    pub(super) fn bytes(&self) -> u64 {
        self.bytes
    }
    fn summed(&self) -> u64 {
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
        // The chain resolves the samples itself (`Pass::Resolve`).
        let resolves = config.post.on && self.samples > 1;
        let wanted = [
            config.shadows.then_some(Pass::Shadows),
            config.transmits.then_some(Pass::Behind),
            Some(Pass::Main),
            #[cfg(feature = "post")]
            resolves.then_some(Pass::Resolve),
            #[cfg(feature = "post")]
            (config.post.hdr > 0).then_some(Pass::AppHdr),
            #[cfg(feature = "post")]
            config.post.bloom.then_some(Pass::Bloom),
            #[cfg(feature = "post")]
            config.post.on.then_some(Pass::Final),
            #[cfg(feature = "post")]
            (config.post.display > 0).then_some(Pass::AppDisplay),
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
        let read = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let scene = Desc {
            samples: 1,
            usage: read,
            ..target(config.size, self.format)
        };
        let mut used: Vec<(Resource, Desc, usize, usize)> = Vec::new();
        for (at, pass) in self.passes.iter().enumerate() {
            for &resource in pass.reads().iter().chain(pass.writes()) {
                let desc = match resource {
                    Resource::Colour if resolves => Desc {
                        usage: read,
                        ..target(config.size, self.format)
                    },
                    Resource::Colour if self.samples > 1 => target(config.size, self.format),
                    Resource::Depth => target(config.size, DEPTH),
                    Resource::Scene if config.post.on => scene,
                    Resource::Scene2 if config.post.hdr > 0 => scene,
                    Resource::Shown if config.post.display > 0 => Desc {
                        format: SCENE,
                        ..scene
                    },
                    Resource::Shown2 if config.post.display > 1 => Desc {
                        format: SCENE,
                        ..scene
                    },
                    Resource::Bloom if config.post.bloom => {
                        bloom_desc(config.size, bloom_format(device))
                    }
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
                Resource::Scene => "3D scene",
                Resource::Bloom => "3D bloom",
                Resource::Scene2 | Resource::Shown | Resource::Shown2 => "3D post",
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
        let scene = view(Resource::Scene);
        let others = [Resource::Scene2, Resource::Shown, Resource::Shown2].map(view);
        let bloom = match used.iter().position(|(r, ..)| *r == Resource::Bloom) {
            Some(at) => {
                let texture = &textures[slots[at]].as_ref().expect("taken").texture;
                (0..used[at].1.levels)
                    .map(|level| {
                        texture.create_view(&wgpu::TextureViewDescriptor {
                            base_mip_level: level,
                            mip_level_count: Some(1),
                            ..Default::default()
                        })
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        self.targets = Some(Targets {
            colour,
            depth: depth.expect("the meshes' pass tests depth"),
            scene,
            bloom,
            stored: resolves,
            others,
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
        self.bytes = self.summed();
        self.compiled += 1;
        behind_made
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::device;
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

    #[test]
    fn a_list_is_compiled_when_its_configuration_changes_and_rests_holding_nothing() {
        let device = device();
        let mut graph = Graph::new(wgpu::TextureFormat::Bgra8UnormSrgb, 4);
        let plain = Config {
            size: (64, 32),
            shadows: false,
            transmits: false,
            post: Chain::default(),
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

    #[cfg(feature = "post")]
    #[test]
    fn the_post_chain_runs_after_the_meshes_from_a_scene_of_its_own() {
        let device = device();
        let mut graph = Graph::new(SCENE, 1);
        let config = |bloom| Config {
            size: (64, 32),
            shadows: true,
            transmits: false,
            post: Chain {
                on: true,
                bloom,
                ..Default::default()
            },
        };
        graph.prepare(&device, config(true));
        assert_eq!(
            graph.passes().iter().collect::<Vec<_>>(),
            [Pass::Shadows, Pass::Main, Pass::Bloom, Pass::Final]
        );
        let targets = graph.targets.as_ref().unwrap();
        assert!(targets.scene.is_some() && targets.colour.is_none());
        // Bloom from 32 by 16 down to 2 by 1 (16 is 2 to the 4th: 4 steps).
        assert_eq!(targets.bloom.len(), 4);
        let texel = u64::from(bloom_format(&device).block_copy_size(None).unwrap());
        let steps: u64 = (0..4).map(|l| (32u64 >> l) * (16u64 >> l) * texel).sum();
        assert_eq!(graph.bytes(), 64 * 32 * (8 + 4) + steps);
        graph.prepare(&device, config(false));
        assert_eq!(
            graph.passes().iter().collect::<Vec<_>>(),
            [Pass::Shadows, Pass::Main, Pass::Final]
        );
        assert!(graph.targets.as_ref().unwrap().bloom.is_empty());
        assert_eq!(graph.bytes(), 64 * 32 * (8 + 4));
    }
}
