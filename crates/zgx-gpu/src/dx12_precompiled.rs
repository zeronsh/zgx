//! draw.wgsl's stages on Direct3D 12, compiled by FXC ahead of time
//! (`dx12/`) and handed to wgpu as bytecode, where wgpu would otherwise
//! translate them with naga and compile them with FXC at every launch: a
//! window's first frame paid 64 ms for that the first time and 9 ms on every
//! launch after (its HLSL rebuilt and looked up in the shader cache), on an
//! RTX 4090.
//!
//! Each stage is the HLSL wgpu-hal writes for it (`dx12_fill::stage_hlsl`)
//! compiled with the flags wgpu-hal gives FXC without the instance's `DEBUG`,
//! so the bytecode is the same wgpu would compile, and frames the same bytes:
//! taken only where wgpu would compile these with FXC and those flags
//! (`precompiled_quads`). `precompiled_shaders_match` (Windows) compiles them
//! again, and `tests/precompiled_d3d12.rs` checks the bytecode against what
//! wgpu compiles itself and the frames against wgpu's.
#[cfg(test)]
use crate::dx12_fill::{self, Layout};
#[cfg(test)]
use wgpu::naga::ShaderStage;

/// draw.wgsl's stages the quad pipelines draw with, the bytecode of each.
/// A vertex stage is written for the first fragment stage it is drawn with
/// (naga writes a vertex stage's outputs for its fragment's inputs); the
/// others it is drawn with read the same
/// (`each_vertex_stage_serves_its_fragments`).
pub(crate) const STAGES: [Stage; 6] = [
    Stage {
        entry: "vs",
        fragment: Some("fs"),
        code: include_bytes!("dx12/draw_vs.dxbc"),
    },
    Stage {
        entry: "vs_basic",
        fragment: Some("fs_basic"),
        code: include_bytes!("dx12/draw_vs_basic.dxbc"),
    },
    Stage {
        entry: "fs",
        fragment: None,
        code: include_bytes!("dx12/draw_fs.dxbc"),
    },
    Stage {
        entry: "fs_both",
        fragment: None,
        code: include_bytes!("dx12/draw_fs_both.dxbc"),
    },
    Stage {
        entry: "fs_basic",
        fragment: None,
        code: include_bytes!("dx12/draw_fs_basic.dxbc"),
    },
    Stage {
        entry: "fs_basic_both",
        fragment: None,
        code: include_bytes!("dx12/draw_fs_basic_both.dxbc"),
    },
];

/// A stage of draw.wgsl: its entry point, for a vertex stage the fragment
/// stage it is written for, and its bytecode.
pub(crate) struct Stage {
    pub entry: &'static str,
    #[cfg_attr(not(test), allow(dead_code))]
    pub fragment: Option<&'static str>,
    pub code: &'static [u8],
}

#[cfg(test)]
impl Stage {
    fn stage(&self) -> ShaderStage {
        if self.fragment.is_some() {
            ShaderStage::Vertex
        } else {
            ShaderStage::Fragment
        }
    }
    /// Its HLSL, as wgpu-hal writes it, and the entry point's name there.
    fn hlsl(&self) -> Result<(String, String), String> {
        dx12_fill::stage_hlsl(
            include_str!("draw.wgsl"),
            self.stage(),
            self.entry,
            self.fragment,
            Layout::Quads,
        )
    }
    /// FXC's target for it: shader model 5.1, wgpu-hal's with FXC.
    fn target(&self) -> &'static str {
        match self.stage() {
            ShaderStage::Vertex => "vs_5_1",
            _ => "ps_5_1",
        }
    }
}

/// The flags wgpu-hal compiles with FXC when the instance has no `DEBUG`
/// (`compile_fxc`): strict.
#[cfg(test)]
const FLAGS: u32 = 1 << 11;

/// draw.wgsl's stages as wgpu modules, by entry point.
pub(crate) struct Stages(Vec<(&'static str, wgpu::ShaderModule)>);

impl Stages {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        Self(
            STAGES
                .iter()
                .map(|stage| {
                    // SAFETY: FXC's bytecode of the HLSL wgpu-hal writes for
                    // this stage and the quads' layout, with its flags
                    // (checked by `precompiled_shaders_match` and
                    // tests/precompiled_d3d12.rs).
                    let module = unsafe {
                        device.create_shader_module_passthrough(
                            wgpu::ShaderModuleDescriptorPassthrough {
                                label: Some("quads"),
                                dxil: Some(stage.code.into()),
                                ..Default::default()
                            },
                        )
                    };
                    (stage.entry, module)
                })
                .collect(),
        )
    }
    /// The module of `entry`.
    pub(crate) fn get(&self, entry: &str) -> &wgpu::ShaderModule {
        &self
            .0
            .iter()
            .find(|(name, _)| *name == entry)
            .unwrap_or_else(|| panic!("draw.wgsl's `{entry}` isn't compiled ahead"))
            .1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each stage's bytecode is FXC's of its HLSL today:
    /// `ZGX_BLESS=1 cargo test -p zgx-gpu --lib precompiled_shaders` writes
    /// them again (Windows: FXC). With `ZGX_SHADER_OUT` set, stale ones are
    /// also written there (CI keeps them, for a machine without FXC to take).
    #[test]
    fn precompiled_shaders_match() {
        let bless = std::env::var_os("ZGX_BLESS").is_some_and(|v| v != "0");
        let out = std::env::var_os("ZGX_SHADER_OUT").map(std::path::PathBuf::from);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/dx12");
        let mut stale = Vec::new();
        for stage in &STAGES {
            let (hlsl, name) = stage.hlsl().unwrap();
            let code = crate::fxc_child::compile_here(&crate::fxc_child::Job {
                source: &hlsl,
                entry_point: &name,
                target: stage.target(),
                flags: FLAGS,
            })
            .unwrap_or_else(|e| panic!("FXC failed on `{}`: {e}", stage.entry));
            let file = format!("draw_{}.dxbc", stage.entry);
            if bless {
                std::fs::write(dir.join(&file), &code).unwrap();
            } else if code != stage.code {
                if let Some(out) = &out {
                    std::fs::create_dir_all(out).unwrap();
                    std::fs::write(out.join(&file), &code).unwrap();
                }
                stale.push(file);
            }
        }
        assert!(
            stale.is_empty(),
            "src/dx12/{stale:?} are not their WGSL's: \
             `ZGX_BLESS=1 cargo test -p zgx-gpu --lib precompiled_shaders` (Windows)"
        );
    }

    /// A vertex stage written for its first fragment stage is the one
    /// written for every other it is drawn with (`quad_pipeline`), so one
    /// bytecode serves them all.
    #[test]
    fn each_vertex_stage_serves_its_fragments() {
        for (vertex, fragments) in [
            ("vs", ["fs", "fs_both"]),
            ("vs_basic", ["fs_basic", "fs_basic_both"]),
        ] {
            let written = |fragment| {
                dx12_fill::stage_hlsl(
                    include_str!("draw.wgsl"),
                    ShaderStage::Vertex,
                    vertex,
                    Some(fragment),
                    Layout::Quads,
                )
                .unwrap()
            };
            assert_eq!(written(fragments[0]), written(fragments[1]), "{vertex}");
        }
    }
}
