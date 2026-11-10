//! A frame's temporary textures: made from a pool when the pass list is
//! compiled, shared between resources whose lifetimes in the list don't
//! overlap, and given back whole when it's compiled again or the world
//! rests (design-3d-aaa.md, 4.3). Targets are written by render passes only:
//! on Metal any blit into a texture opens 8 MB driver blocks.
use zgx_gpu::wgpu;

/// What a temporary texture is: two with equal ones can be the same texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Desc {
    pub(super) size: (u32, u32),
    pub(super) format: wgpu::TextureFormat,
    pub(super) samples: u32,
    pub(super) levels: u32,
    pub(super) usage: wgpu::TextureUsages,
}
impl Desc {
    /// Its bytes on the device, every level and sample (block-compressed
    /// formats aren't targets).
    pub(super) fn bytes(&self) -> u64 {
        let texel = u64::from(self.format.block_copy_size(None).unwrap_or(4));
        let (mut width, mut height) = (u64::from(self.size.0), u64::from(self.size.1));
        let mut bytes = 0;
        for _ in 0..self.levels {
            bytes += width * height * texel * u64::from(self.samples);
            (width, height) = ((width / 2).max(1), (height / 2).max(1));
        }
        bytes
    }
}

/// A texture of the pool's, with what it was made as.
pub(super) struct Temporary {
    pub(super) desc: Desc,
    pub(super) texture: wgpu::Texture,
}

/// Textures given back, kept until the next compile takes them or lets
/// them go.
#[derive(Default)]
pub(super) struct Pool {
    free: Vec<Temporary>,
}
impl Pool {
    /// A texture as `desc` says: one given back if there is one, else made.
    pub(super) fn take(&mut self, device: &wgpu::Device, desc: Desc, label: &str) -> Temporary {
        if let Some(at) = self.free.iter().position(|t| t.desc == desc) {
            return self.free.swap_remove(at);
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: desc.size.0,
                height: desc.size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: desc.levels,
            sample_count: desc.samples,
            dimension: wgpu::TextureDimension::D2,
            format: desc.format,
            usage: desc.usage,
            view_formats: &[],
        });
        Temporary { desc, texture }
    }
    /// `temporary` back, for a compile to take again.
    pub(super) fn give(&mut self, temporary: Temporary) {
        self.free.push(temporary);
    }
    /// Lets go of what no compile took.
    pub(super) fn clear(&mut self) {
        self.free.clear();
    }
}

/// Which texture each resource gets: resources are `(desc, first, last)`,
/// the passes using them first and last, in the list's order; a resource
/// shares the texture of an earlier one of its `desc` whose last pass came
/// before its first. Returns each resource's slot and how many slots.
pub(super) fn assign(resources: &[(Desc, usize, usize)]) -> (Vec<usize>, usize) {
    // Each slot's desc and the last pass using it so far.
    let mut slots: Vec<(Desc, usize)> = Vec::new();
    let mut by_first: Vec<usize> = (0..resources.len()).collect();
    by_first.sort_by_key(|&i| resources[i].1);
    let mut assigned = vec![0; resources.len()];
    for i in by_first {
        let (desc, first, last) = resources[i];
        let free = slots
            .iter()
            .position(|&(slot, until)| slot == desc && until < first);
        let slot = match free {
            Some(slot) => {
                slots[slot].1 = last;
                slot
            }
            None => {
                slots.push((desc, last));
                slots.len() - 1
            }
        };
        assigned[i] = slot;
    }
    (assigned, slots.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(side: u32) -> Desc {
        Desc {
            size: (side, side),
            format: wgpu::TextureFormat::Rgba16Float,
            samples: 1,
            levels: 1,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        }
    }

    #[test]
    fn resources_alike_share_a_texture_only_when_their_lifetimes_dont_overlap() {
        let (a, b) = (desc(64), desc(32));
        // 0 and 2 alike and apart; 1 alike but overlapping 0; 3 different.
        // Slots go in the order of first use.
        let (slots, count) = assign(&[(a, 0, 1), (a, 1, 2), (a, 2, 3), (b, 0, 3)]);
        assert_eq!(slots, vec![0, 2, 0, 1]);
        assert_eq!(count, 3);
        // One pass's end and the next one's start don't overlap.
        assert_eq!(assign(&[(a, 0, 0), (a, 1, 1)]), (vec![0, 0], 1));
    }

    #[test]
    fn a_textures_bytes_count_every_level_and_sample() {
        let mut d = desc(4);
        assert_eq!(d.bytes(), 4 * 4 * 8);
        d.levels = 3;
        assert_eq!(d.bytes(), (16 + 4 + 1) * 8);
        d.levels = 1;
        d.samples = 4;
        d.format = wgpu::TextureFormat::Depth32Float;
        assert_eq!(d.bytes(), 16 * 4 * 4);
    }
}
