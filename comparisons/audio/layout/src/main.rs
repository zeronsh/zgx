//! Planar against interleaved buffers for the mixer's two inner loops: a
//! stereo voice into its bus (gains settled), and a bus into its parent.
//! Instructions on this thread for 64 voices over 10 s in 128-frame blocks.
use std::hint::black_box;
const N: usize = 128;
const VOICES: usize = 64;
const BLOCKS: usize = 10 * 48_000 / N;
fn count() -> u64 {
    unsafe extern "C" { fn thread_selfcounts(kind: i32, buffer: *mut u64, size: usize) -> i32; }
    let mut c = [0u64; 2];
    unsafe { thread_selfcounts(1, c.as_mut_ptr(), 16) };
    c[0]
}
#[inline(never)]
fn planar(src: &[f32], at: usize, sl: &mut [f32; N], sr: &mut [f32; N], bl: &mut [f32; N], br: &mut [f32; N], gl: f32, gr: f32) {
    // the copy the resampler does at the sound's own rate, then the mix
    for ((l, r), p) in sl.iter_mut().zip(sr.iter_mut()).zip(src[at * 2..(at + N) * 2].as_chunks::<2>().0) { [*l, *r] = *p; }
    for i in 0..N { bl[i] += sl[i] * gl; br[i] += sr[i] * gr; }
}
#[inline(never)]
fn interleaved(src: &[f32], at: usize, s: &mut [f32; 2 * N], b: &mut [f32; 2 * N], gl: f32, gr: f32) {
    s.copy_from_slice(&src[at * 2..(at + N) * 2]);
    for (o, i) in b.as_chunks_mut::<2>().0.iter_mut().zip(s.as_chunks::<2>().0) { o[0] += i[0] * gl; o[1] += i[1] * gr; }
}
#[inline(never)]
fn bus_planar(bl: &[f32; N], br: &[f32; N], pl: &mut [f32; N], pr: &mut [f32; N], g: f32) {
    for i in 0..N { pl[i] += bl[i] * g; pr[i] += br[i] * g; }
}
#[inline(never)]
fn bus_interleaved(b: &[f32; 2 * N], p: &mut [f32; 2 * N], g: f32) {
    for i in 0..2 * N { p[i] += b[i] * g; }
}
fn main() {
    let src: Vec<f32> = (0..96_000).map(|i| ((i * 7919) % 1000) as f32 / 1000. - 0.5).collect();
    let (mut sl, mut sr, mut bl, mut br, mut pl, mut pr) = ([0f32; N], [0f32; N], [0f32; N], [0f32; N], [0f32; N], [0f32; N]);
    let (mut s, mut b, mut p) = ([0f32; 2 * N], [0f32; 2 * N], [0f32; 2 * N]);
    let mut best = [u64::MAX; 4];
    for _ in 0..5 {
        for arm in 0..4 {
            let before = count();
            let mut at = 0;
            for _ in 0..BLOCKS {
                for v in 0..VOICES {
                    let g = black_box(0.01 + v as f32 * 1e-4);
                    match arm {
                        0 => planar(&src, at, &mut sl, &mut sr, &mut bl, &mut br, g, g * 0.5),
                        1 => interleaved(&src, at, &mut s, &mut b, g, g * 0.5),
                        2 => bus_planar(&bl, &br, &mut pl, &mut pr, g),
                        _ => bus_interleaved(&b, &mut p, g),
                    }
                }
                at = (at + N) % (48_000 - N);
            }
            best[arm] = best[arm].min(count() - before);
        }
    }
    black_box((&bl, &br, &b, &pl, &pr, &p));
    let per = |x: u64| x as f64 / (BLOCKS * VOICES * N) as f64;
    println!("voice into bus, per frame: planar {:.2}, interleaved {:.2}", per(best[0]), per(best[1]));
    println!("bus into parent, per frame: planar {:.2}, interleaved {:.2}", per(best[2]), per(best[3]));
}
