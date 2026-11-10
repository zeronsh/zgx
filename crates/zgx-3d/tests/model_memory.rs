//! What reading a textured model holds: its files, not its textures'
//! pixels, which decode as a renderer uploads them (a few at a time). A
//! test binary of its own: a counting allocator measures the heap.
#![cfg(feature = "gltf")]
use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
};
use zgx_3d::Model;

/// The system's allocator, counting live bytes and their peak.
struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn grew(by: usize) {
    let now = LIVE.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        grew(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if size > layout.size() {
            grew(size - layout.size());
        } else {
            LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The CRC of `bytes` (a PNG chunk's).
fn crc(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                0xedb8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A `side` by `side` RGBA PNG of `seed`'s colour, its data stored
/// uncompressed (zlib's stored blocks): its file as large as its pixels.
fn png(side: u32, seed: u8) -> Vec<u8> {
    let row = |_: u32| {
        let mut row = vec![0u8]; // no filter
        row.extend((0..side).flat_map(|x| [seed, (x as u8) ^ seed, 255 - seed, 255]));
        row
    };
    let raw: Vec<u8> = (0..side).flat_map(row).collect();
    let mut zlib = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65_535).enumerate() {
        let last = (i + 1) * 65_535 >= raw.len();
        zlib.push(u8::from(last));
        let len = block.len() as u16;
        zlib.extend(len.to_le_bytes());
        zlib.extend((!len).to_le_bytes());
        zlib.extend(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    zlib.extend((b << 16 | a).to_be_bytes());
    let mut file = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        file.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend(data);
        file.extend(&body);
        file.extend(crc(&body).to_be_bytes());
    };
    let mut header = side.to_be_bytes().to_vec();
    header.extend(side.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]); // 8 bits, RGBA
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &zlib);
    chunk(b"IEND", &[]);
    file
}

/// A glTF of `count` quads, each with a texture of its own (`t{i}.png`),
/// and the files it names.
fn model(count: usize, side: u32) -> (String, HashMap<String, Vec<u8>>) {
    let mut files = HashMap::new();
    let mut bin = Vec::new();
    for p in [[0f32, 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]] {
        p.iter().for_each(|v| bin.extend(v.to_le_bytes()));
    }
    for t in [[0f32, 1.], [1., 1.], [1., 0.], [0., 0.]] {
        t.iter().for_each(|v| bin.extend(v.to_le_bytes()));
    }
    for i in [0u16, 1, 2, 0, 2, 3] {
        bin.extend(i.to_le_bytes());
    }
    let len = bin.len();
    files.insert("quad.bin".to_owned(), bin);
    let list = |f: &dyn Fn(usize) -> String| (0..count).map(f).collect::<Vec<_>>().join(",");
    for i in 0..count {
        files.insert(format!("t{i}.png"), png(side, i as u8));
    }
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[{roots}]}}],
"nodes":[{nodes}],"meshes":[{meshes}],"materials":[{materials}],"textures":[{textures}],"images":[{images}],
"buffers":[{{"uri":"quad.bin","byteLength":{len}}}],
"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":48}},{{"buffer":0,"byteOffset":48,"byteLength":32}},{{"buffer":0,"byteOffset":80,"byteLength":12}}],
"accessors":[{{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},{{"bufferView":1,"componentType":5126,"count":4,"type":"VEC2"}},{{"bufferView":2,"componentType":5123,"count":6,"type":"SCALAR"}}]}}"#,
        roots = list(&|i| i.to_string()),
        nodes = list(&|i| format!(r#"{{"mesh":{i},"translation":[{},0,0]}}"#, i as f32 * 1.1)),
        meshes = list(&|i| format!(
            r#"{{"primitives":[{{"attributes":{{"POSITION":0,"TEXCOORD_0":1}},"indices":2,"material":{i}}}]}}"#
        )),
        materials = list(&|i| format!(
            r#"{{"pbrMetallicRoughness":{{"baseColorTexture":{{"index":{i}}}}}}}"#
        )),
        textures = list(&|i| format!(r#"{{"source":{i}}}"#)),
        images = list(&|i| format!(r#"{{"uri":"t{i}.png"}}"#)),
    );
    (json, files)
}

#[test]
fn reading_a_textured_model_holds_its_files_not_its_pixels() {
    // 32 textures of 512²: 1 MB of pixels each (their files as large).
    let (count, side) = (32, 512);
    let (json, files) = model(count, side);
    let pixels = side as usize * side as usize * 4;
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let model = Model::from_bytes_with(json.as_bytes(), |uri| files.get(uri).cloned());
    let model = model.expect("the model");
    let peak = PEAK.load(Ordering::Relaxed) - before;
    // The files it keeps (measured: and 1.4 MB more), not the pixels too
    // (measured before: the files and all 32 textures' pixels).
    let file_bytes: usize = files.values().map(Vec::len).sum();
    assert!(
        peak < file_bytes + 4 * pixels,
        "reading held {peak} bytes at its peak; the files are {file_bytes}, the pixels {}",
        count * pixels
    );
    drop(model);
}
