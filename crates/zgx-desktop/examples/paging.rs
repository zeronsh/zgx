//! A scroll view that comes to rest on whole pages, beside two that don't, for
//! `scripts/paging_smoke.py` and `scripts/windows_touch_smoke.py`: each page, row
//! and column a colour of its own, so a pixel says where a view is.
use zgx::{compose::prelude::*, scene::Color};
use zgx_desktop::{Application, WindowOptions};

/// The width of a page, the height of the views.
const PAGE: f32 = 300.;
const HEIGHT: f32 = 300.;
const PAGE_COLOURS: [u32; 4] = [0xc03030, 0x30a040, 0x3050c0, 0xc0a020];
/// How long a stripe is, in the views that don't page: under a page, so that
/// resting on a whole page's multiple would show.
const STRIPE: f32 = 40.;

fn stripes(count: usize, make: impl Fn(Color) -> View) -> Vec<View> {
    (0..count)
        .map(|i| {
            let level = (40 + (i * 200 / count)) as u8;
            make(Color(level, level, 255 - level, 255))
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: "zgx paging".into(),
            width: (PAGE * 3.) as f64,
            height: HEIGHT as f64,
            ..Default::default()
        })
        .run(move |cx| {
            let (paged, down, across) = (
                cx.ui.signal(0.0_f32),
                cx.ui.signal(0.0_f32),
                cx.ui.signal(0.0_f32),
            );
            cx.ui.mount(
                row()
                    .child(
                        scroll_x(paged)
                            .id("paged")
                            .w(PAGE)
                            .h(HEIGHT)
                            .paging(true)
                            .child(row().children(
                                PAGE_COLOURS.map(|colour| div().size(PAGE, HEIGHT).bg(rgb(colour))),
                            )),
                    )
                    .child(
                        scroll(down).id("down").w(PAGE).h(HEIGHT).child(
                            column().children(stripes(40, |colour| {
                                div().size(PAGE, STRIPE).bg(colour)
                            })),
                        ),
                    )
                    .child(scroll_x(across).id("across").w(PAGE).h(HEIGHT).child(
                        row().children(stripes(40, |colour| div().size(STRIPE, HEIGHT).bg(colour))),
                    )),
            );
        })
}
