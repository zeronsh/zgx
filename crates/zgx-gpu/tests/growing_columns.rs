//! A container that only grows or shrinks where it stands, painting nothing
//! of its own (a streamed reply's column, every chunk), damages none of its
//! subtree: each child whose box changes damages itself. Every frame here
//! is checked against a fresh full render, where the growth moves children
//! (stretched, end-, centre- and space-between-justified, sized by
//! percentages, placed from the bottom), aligns them to an edge that moves,
//! or grows a scroll view's content under its scrollbar.
use zgx::compose::prelude::*;
use zgx::reactive::Signal;
use zgx::scene::Rect;
use zgx::widgets::Ui;
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
const W: f32 = 320.;
const H: f32 = 240.;

/// A block of the reply: a coloured box with a line of text.
fn block(n: usize) -> View {
    div()
        .w(120. + (n * 17 % 60) as f32)
        .h(14. + (n * 5 % 9) as f32)
        .bg(rgb(0x203040 + (n as u32 * 0x0a0b0c) % 0x606060))
        .child(
            text(format!("Block {n}"))
                .text_size(10.)
                .text_color(rgb(0xffffff)),
        )
}

/// `view` of `count` grown frame by frame through `counts`: every frame
/// drawn with its damage equals a fresh renderer's full frame.
fn grow(case: &str, view: impl FnOnce(Signal<usize>, Signal<f32>) -> View, counts: &[usize]) {
    let mut gpu = GpuRenderer::new(W as u32, H as u32).unwrap();
    let mut ui = Ui::new(W, H);
    let count = ui.signal(counts[0]);
    let offset = ui.signal(0_f32);
    ui.mount(view(count.clone(), offset));
    for &n in counts {
        count.set(n);
        ui.prepare_frame();
        let damage = ui.scene.borrow_mut().flush().damage;
        gpu.render(&ui.scene.borrow(), &damage).unwrap();
        let mut fresh = GpuRenderer::new(W as u32, H as u32).unwrap();
        fresh
            .render(&ui.scene.borrow(), &[Rect::new(0., 0., W, H)])
            .unwrap();
        let (a, b) = (gpu.readback().unwrap(), fresh.readback().unwrap());
        let differs = (0..(W * H) as usize).find(|i| a[i * 4..][..4] != b[i * 4..][..4]);
        assert!(
            differs.is_none(),
            "{case}, {n} blocks: at {:?} {:?} where a full frame shows {:?}",
            differs.map(|i| (i % W as usize, i / W as usize)),
            differs.map(|i| &a[i * 4..][..4]),
            differs.map(|i| &b[i * 4..][..4]),
        );
    }
}

fn blocks(count: Signal<usize>) -> View {
    keyed(
        move || (0..count.get()).collect::<Vec<_>>(),
        |n, _| block(n),
    )
}

/// How a case lays out its children along its main axis.
type Arrange = fn(View) -> View;

const STEPS: &[usize] = &[1, 2, 3, 5, 6, 8, 9, 7, 4, 10, 11];

#[test]
fn a_streamed_column_repaints_what_a_full_frame_shows() {
    let _gpu_fixture = gpu_fixture();
    grow(
        "streamed",
        |count, _| {
            div().w(W).h(H).bg(rgb(0x101010)).child(
                column()
                    .p(6.)
                    .gap(2.)
                    .child(column().gap(2.).child(blocks(count))),
            )
        },
        STEPS,
    );
}

#[test]
fn children_a_stretched_column_moves_as_it_grows_repaint() {
    let _gpu_fixture = gpu_fixture();
    // A row grows taller as its left column streams; its right column,
    // stretched to the row's height, grows where it stands, and moves or
    // sizes what it holds by that height.
    let right = |justify: fn(View) -> View| {
        justify(
            column()
                .w(150.)
                .child(div().size(40., 10.).bg(rgb(0xff4040)))
                .child(div().size(60., 10.).bg(rgb(0x40ff40)))
                .child(div().w(30.).h_percent(25.).bg(rgb(0x4040ff)))
                .child(
                    div()
                        .absolute()
                        .bottom(0.)
                        .right(0.)
                        .size(20., 8.)
                        .bg(rgb(0xffff40)),
                ),
        )
    };
    let cases: [(&str, Arrange); 4] = [
        ("end", |v| v.justify_end()),
        ("centre", |v| v.justify_center()),
        ("space between", |v| v.justify_between()),
        ("start", |v| v.justify_start()),
    ];
    for (case, justify) in cases {
        grow(
            case,
            move |count, _| {
                div().w(W).h(H).bg(rgb(0x101010)).child(
                    row()
                        .items_stretch()
                        .child(column().w(160.).gap(2.).child(blocks(count)))
                        .child(right(justify).relative()),
                )
            },
            STEPS,
        );
    }
}

#[test]
fn children_aligned_to_a_growing_edge_repaint() {
    let _gpu_fixture = gpu_fixture();
    // Right-aligned and centred children of a column that widens as wider
    // blocks stream into it.
    for (case, items) in [
        ("right", (|v| v.items_end()) as Arrange),
        ("centre", |v| v.items_center()),
    ] {
        grow(
            case,
            move |count, _| {
                div().w(W).h(H).bg(rgb(0x101010)).child(items(
                    column()
                        .child(div().size(24., 10.).bg(rgb(0xff8040)))
                        .child(blocks(count))
                        .child(div().size(16., 10.).bg(rgb(0x40c0ff))),
                ))
            },
            STEPS,
        );
    }
}

#[test]
fn a_scroll_view_s_growing_content_repaints_under_its_scrollbar() {
    let _gpu_fixture = gpu_fixture();
    let steps: Vec<usize> = (1..=24).chain([20, 26, 30]).collect();
    grow(
        "scroll",
        |count, offset| {
            div().w(W).h(H).bg(rgb(0x101010)).child(
                scroll(offset)
                    .w(200.)
                    .h(120.)
                    .bg(rgb(0x182028))
                    .child(column().gap(2.).child(blocks(count))),
            )
        },
        &steps,
    );
}
