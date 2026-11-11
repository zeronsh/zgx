//! A record of the probes' updates, for telling where two runs of the
//! same scene part (`ZGX_GI_DUMP=<folder>`; nothing is made or copied
//! without it). Each world's probes (in the order a process makes them)
//! write `world-<n>/` in the folder: `layout.txt`; for each of the first
//! `ZGX_GI_DUMP_FRAMES` updates (all, by default) its rays as traced or
//! read from faces (`frame-<f>-rays.bin`: each of this update's probes'
//! rays, RGB and distance as `f32`, negative behind a face) and both
//! atlases after the blend (`frame-<f>-light.bin`, `frame-<f>-depth.bin`:
//! `Rgba16Float` rows); and in `hashes.txt` a line an update (its round,
//! first probe and count, and hashes of what the host gave it: the
//! update's uniform and the lights) and a line a probe (hashes of its
//! rays, where updated, and of its two tiles). Two runs' `hashes.txt`
//! differ first at the update and probe where they part: rays differing
//! there are the trace's (or the structure it ran through); rays alike
//! and tiles not, the blend's. `tests/gi.rs`'s `probe_dumps_part`
//! compares two folders and prints the first differing probe's values.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use zgx_gpu::wgpu;

/// Whether this process records its probes (and so makes what it copies
/// from copyable).
pub(super) fn requested() -> bool {
    std::env::var_os("ZGX_GI_DUMP").is_some()
}

static WORLDS: AtomicU32 = AtomicU32::new(0);

pub(super) struct Dump {
    device: wgpu::Device,
    folder: PathBuf,
    /// Updates recorded, and the most to record.
    frame: u32,
    most: u32,
    /// Copies made, read once their frame is submitted.
    pending: Vec<Pending>,
}

struct Pending {
    line: String,
    frame: u32,
    first: u32,
    count: u32,
    total: u32,
    rays_per_probe: u32,
    rays: wgpu::Buffer,
    /// Each atlas's copy, its size in texels, its tile's side and its
    /// copy's bytes a row.
    atlases: [(wgpu::Buffer, [u32; 2], u32, u32); 2],
}

/// What an update gives the record: the uniform and lights the host
/// wrote, the rays, both atlases (texture, size, tile).
pub(super) struct Update<'a> {
    pub uniform: &'a [u8],
    pub lights: &'a [u8],
    pub round: u32,
    pub first: u32,
    pub count: u32,
    pub total: u32,
    pub rays_per_probe: u32,
    pub rays: &'a wgpu::Buffer,
    pub atlases: [(&'a wgpu::Texture, [u32; 2], u32); 2],
}

/// FNV-1a.
fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

impl Dump {
    pub(super) fn from_env(device: &wgpu::Device) -> Option<Self> {
        let folder = PathBuf::from(std::env::var_os("ZGX_GI_DUMP")?);
        let world = WORLDS.fetch_add(1, Ordering::Relaxed);
        let folder = folder.join(format!("world-{world}"));
        std::fs::create_dir_all(&folder).ok()?;
        let _ = std::fs::remove_file(folder.join("hashes.txt"));
        let most = std::env::var("ZGX_GI_DUMP_FRAMES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(u32::MAX);
        Some(Self {
            device: device.clone(),
            folder,
            frame: 0,
            most,
            pending: Vec::new(),
        })
    }

    /// Copies of this update's rays and atlases into `encoder` (after its
    /// blend), read by the next `flush`.
    pub(super) fn record(&mut self, encoder: &mut wgpu::CommandEncoder, update: Update) {
        if self.frame >= self.most {
            return;
        }
        let frame = self.frame;
        self.frame += 1;
        if frame == 0 {
            let [(_, light, light_tile), (_, depth, depth_tile)] = update.atlases;
            let _ = std::fs::write(
                self.folder.join("layout.txt"),
                format!(
                    "total {} rays {} light {} {} {} depth {} {} {} columns 64\n",
                    update.total,
                    update.rays_per_probe,
                    light[0],
                    light[1],
                    light_tile,
                    depth[0],
                    depth[1],
                    depth_tile,
                ),
            );
        }
        let read = |size: u64| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("3D probe record"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        };
        let size = u64::from(update.count * update.rays_per_probe) * 16;
        let rays = read(size.max(16));
        if size > 0 {
            encoder.copy_buffer_to_buffer(update.rays, 0, &rays, 0, size);
        }
        let atlases = update.atlases.map(|(texture, size, tile)| {
            let row = (size[0] * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
            let buffer = read(u64::from(row * size[1]));
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
            );
            (buffer, size, tile, row)
        });
        self.pending.push(Pending {
            line: format!(
                "frame {frame} round {} first {} count {} uniform {:016x} lights {:016x}",
                update.round,
                update.first,
                update.count,
                hash(update.uniform),
                hash(update.lights),
            ),
            frame,
            first: update.first,
            count: update.count,
            total: update.total,
            rays_per_probe: update.rays_per_probe,
            rays,
            atlases,
        });
    }

    /// Reads and writes out what was copied (its frames since submitted).
    pub(super) fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        for p in &self.pending {
            p.rays.map_async(wgpu::MapMode::Read, .., |_| {});
            for (buffer, ..) in &p.atlases {
                buffer.map_async(wgpu::MapMode::Read, .., |_| {});
            }
        }
        if self
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .is_err()
        {
            self.pending.clear();
            return;
        }
        let Ok(mut out) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.folder.join("hashes.txt"))
        else {
            self.pending.clear();
            return;
        };
        for p in self.pending.drain(..) {
            let rays = p.rays.slice(..).get_mapped_range().to_vec();
            let rays = &rays[..(p.count * p.rays_per_probe * 16) as usize];
            let _ = std::fs::write(
                self.folder.join(format!("frame-{}-rays.bin", p.frame)),
                rays,
            );
            let tiles = p.atlases.each_ref().map(|(buffer, size, tile, row)| {
                let mapped = buffer.slice(..).get_mapped_range();
                let mut packed = Vec::with_capacity((size[0] * size[1] * 8) as usize);
                for y in 0..size[1] {
                    let start = (y * row) as usize;
                    packed.extend_from_slice(&mapped[start..start + (size[0] * 8) as usize]);
                }
                (packed, size[0], *tile)
            });
            for ((packed, ..), name) in tiles.iter().zip(["light", "depth"]) {
                let _ = std::fs::write(
                    self.folder.join(format!("frame-{}-{name}.bin", p.frame)),
                    packed,
                );
            }
            let _ = writeln!(out, "{}", p.line);
            let per = (p.rays_per_probe * 16) as usize;
            for probe in 0..p.total {
                let slot = (probe + p.total - p.first) % p.total;
                let ray_hash = if slot < p.count {
                    let slot = slot as usize;
                    format!("{:016x}", hash(&rays[slot * per..(slot + 1) * per]))
                } else {
                    "-".repeat(16)
                };
                let tile_hash = |(packed, width, tile): &(Vec<u8>, u32, u32)| {
                    let (x0, y0) = ((probe % 64) * tile, (probe / 64) * tile);
                    let mut h = Vec::with_capacity((tile * tile * 8) as usize);
                    for y in y0..y0 + tile {
                        let start = ((y * width + x0) * 8) as usize;
                        h.extend_from_slice(&packed[start..start + (tile * 8) as usize]);
                    }
                    hash(&h)
                };
                let _ = writeln!(
                    out,
                    "frame {} probe {probe} rays {ray_hash} light {:016x} depth {:016x}",
                    p.frame,
                    tile_hash(&tiles[0]),
                    tile_hash(&tiles[1]),
                );
            }
        }
    }
}

impl Drop for Dump {
    fn drop(&mut self) {
        self.flush();
    }
}
