//! What native views cost beside zgx's own drawing, on iOS: a long list scrolled
//! by a timer (`COST_MODE`): plain, drawn (a zgx switch a row), hosted (a real
//! UISwitch a row), glass (`COST_GLASS` panes over the list), each `COST_ROWS`
//! rows. Measure the process from outside (`scripts/ios/cost.py`).
use std::time::Duration;
use zgx::compose::prelude::*;
use zgx::glass::{GlassStyle, glass};
use zgx::native_view::NativeContent;
use zgx::web_view::native_view;
use zgx_desktop::{Application, WindowOptions};

#[cfg(target_os = "ios")]
fn hosted_switch() -> View {
    use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
    // SAFETY: UIKit's constructor, on the main thread where views build.
    let view: Retained<AnyObject> = unsafe { msg_send![class!(UISwitch), new] };
    native_view(NativeContent::Platform(std::rc::Rc::new(view)))
        .w(63.)
        .h(28.)
}
#[cfg(not(target_os = "ios"))]
fn hosted_switch() -> View {
    let _ = (native_view, |c: NativeContent| c);
    div().w(63.).h(28.)
}

fn drawn_switch() -> View {
    div().w(63.).h(28.).rounded(14.).bg(rgb(0xe5e5ea)).child(
        div()
            .absolute()
            .left(2.)
            .top(2.)
            .w(39.)
            .h(24.)
            .rounded(12.)
            .bg(rgb(0xffffff)),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = zgx::env::var("COST_MODE").unwrap_or_else(|_| "plain".into());
    let number = |name: &str, default: usize| {
        zgx::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    };
    let rows = number("COST_ROWS", 200);
    let panes = number("COST_GLASS", 0);
    // Left where it is: what a window that isn't changing costs.
    let still = zgx::env::var_os("COST_STILL").is_some();
    // Text on each pane, which puts it on the overlay plane.
    let over = zgx::env::var_os("COST_OVER").is_some();
    // Drawing planes stacked between native views: each a pane, and a sheet of
    // paint over the whole window above it.
    let levels = number("COST_LEVELS", 0);
    // How often the scroll moves: less than a refresh, or some refreshes get nothing
    // new, and the gaps measured are this timer's.
    let tick_ms = number("COST_TICK_MS", 8) as u64;
    Application::new()
        .window(WindowOptions::default())
        .run(move |cx| {
            let offset = cx.ui.signal(0.);
            let driver = offset.clone();
            let time = cx.ui.signal(0u32);
            let tick = time.clone();
            cx.tasks.spawn(async move {
                let start = std::time::Instant::now();
                if still {
                    return;
                }
                loop {
                    zgx::timer::sleep(Duration::from_millis(tick_ms)).await;
                    let t = start.elapsed().as_secs_f32();
                    driver.set(((t * 1.2).sin() + 1.) * 2000.);
                    tick.set(tick.get().wrapping_add(1));
                }
            });
            let mut column = column();
            for index in 0..rows {
                let accessory = match mode.as_str() {
                    "drawn" => drawn_switch(),
                    "hosted" => hosted_switch(),
                    _ => div().w(63.).h(28.),
                };
                column = column.child(
                    row()
                        .h(60.)
                        .px(16.)
                        .items_center()
                        .justify_between()
                        .bg(rgb(if index % 2 == 0 { 0xffffff } else { 0xf2f2f7 }))
                        .child(text(format!("Row {index}")))
                        .child(accessory),
                );
            }
            let mut root = div()
                .w_full()
                .h_full()
                .child(scroll(offset).w_full().h_full().child(column));
            for pane in 0..panes {
                root = root.child(
                    glass(|| GlassStyle {
                        radius: 24.,
                        ..Default::default()
                    })
                    .absolute()
                    .left(16.)
                    .right(16.)
                    .top(60. + pane as f32 * 90.)
                    .h(70.)
                    .when(over, |pane| pane.p(20.).child(text("Over the glass"))),
                );
            }
            for level in 0..levels {
                root = root
                    .child(
                        glass(|| GlassStyle {
                            radius: 24.,
                            ..Default::default()
                        })
                        .absolute()
                        .left(16.)
                        .right(16.)
                        .top(120. + level as f32 * 90.)
                        .h(70.),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(0.)
                            .right(0.)
                            .top(0.)
                            .bottom(0.)
                            .bg(rgba(0x0000_0008)),
                    );
            }
            cx.render(root);
        })
}
