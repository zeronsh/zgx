//! Canvas drawing (`canvas`) with Canvas 2D: its commands drawn at the
//! element's size, again as what they read changes.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use zgx::{
    canvas::{
        Brush, Canvas, CanvasText, FillRule, GradientStop, Path, Point, Stroke, TextAnchor,
        TextBaseline,
    },
    compose::{canvas, column},
    reactive::Runtime,
    scene::{Color, Rect},
    style::Styled,
};

wasm_bindgen_test_configure!(run_in_browser);

fn host() -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    host
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// The colour at `x`, `y` (CSS pixels) of the page's canvas in `host`.
fn pixel(host: &web_sys::Element, x: f64, y: f64) -> [u8; 4] {
    let canvas: web_sys::HtmlCanvasElement = host
        .query_selector("canvas")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    let ratio = web_sys::window().unwrap().device_pixel_ratio();
    let ctx: web_sys::CanvasRenderingContext2d = canvas
        .get_context("2d")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    let data = ctx
        .get_image_data((x * ratio) as f64, (y * ratio) as f64, 1., 1.)
        .unwrap()
        .data();
    [data[0], data[1], data[2], data[3]]
}

#[wasm_bindgen_test]
async fn a_canvas_draws_its_commands_and_again_as_they_change() {
    let runtime = Runtime::new();
    let color = runtime.signal(Color(255, 0, 0, 255));
    let read = color.clone();
    let sizes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = sizes.clone();
    let host = host();
    let mounted = zgx_dom::mount(
        &runtime,
        &host,
        column().child(
            canvas(move |(width, height)| {
                seen.borrow_mut().push((width, height));
                let mut drawing = Canvas::new();
                drawing
                    .fill(Path::rectangle(Rect::new(0., 0., 20., 20.)), read.get())
                    .fill(
                        Path::ellipse(Rect::new(40., 0., 20., 20.)).unwrap(),
                        Color(0, 0, 255, 255),
                    )
                    .clip(
                        Path::rectangle(Rect::new(0., 30., 10., 10.)),
                        FillRule::Winding,
                    )
                    .fill(
                        Path::rectangle(Rect::new(0., 30., 40., 10.)),
                        Brush::linear(
                            Point::new(0., 0.),
                            Point::new(10., 0.),
                            [
                                GradientStop {
                                    offset: 0.,
                                    color: Color(0, 255, 0, 255),
                                },
                                GradientStop {
                                    offset: 1.,
                                    color: Color(0, 255, 0, 255),
                                },
                            ],
                        )
                        .unwrap(),
                    )
                    .restore()
                    .stroke(
                        Path::line(Point::new(70., 0.), Point::new(70., 40.)).unwrap(),
                        Color(0, 0, 0, 255),
                        Stroke {
                            width: 4.,
                            ..Default::default()
                        },
                    )
                    .text(CanvasText {
                        text: "zgx".into(),
                        x: 80.,
                        y: 0.,
                        size: 12.,
                        color: Color(0, 0, 0, 255),
                        font: Default::default(),
                        anchor: TextAnchor::Start,
                        baseline: TextBaseline::Top,
                    });
                drawing
            })
            .w(120.)
            .h(40.),
        ),
    );
    sleep(60).await;
    // Drawn at its element's size, once measured.
    assert_eq!(sizes.borrow().last(), Some(&(120., 40.)));
    assert_eq!(pixel(&host, 10., 10.), [255, 0, 0, 255]);
    assert_eq!(pixel(&host, 50., 10.), [0, 0, 255, 255]);
    // The clip keeps the gradient's fill to its 10 points.
    assert_eq!(pixel(&host, 5., 35.), [0, 255, 0, 255]);
    assert_eq!(pixel(&host, 20., 35.)[3], 0);
    assert_eq!(pixel(&host, 70., 20.), [0, 0, 0, 255]);
    // Text drawn: something dark where it is.
    let inked = (80..100).any(|x| pixel(&host, x as f64, 6.)[3] > 0);
    assert!(inked);
    // What it reads changes: drawn again.
    color.set(Color(0, 128, 0, 255));
    zgx_dom::flush();
    assert_eq!(pixel(&host, 10., 10.), [0, 128, 0, 255]);
    mounted.unmount();
    host.remove();
}
