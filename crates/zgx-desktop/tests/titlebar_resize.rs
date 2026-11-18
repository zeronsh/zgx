//! macOS, on the main thread (`fn main`): an inset title bar's buttons
//! (`WindowInfo::controls`, which a title bar laid out around the traffic
//! lights follows) and its area as zgx reads them when winit reports a
//! resize. A window snapped by a window manager (Raycast, Rectangle: its
//! frame set through Accessibility, AXPosition then AXSize) has its content
//! view resized before AppKit moves the title bar to the new top, and winit
//! reports the resize from the content view: read against it, the lights
//! stood the height's change down the window (Zeron's title bar row drew
//! over the sidebar, 900 points down) and the bar was that change tall.
//!
//! The heights come from the screen's visible frame (what AppKit keeps an
//! on-screen window within, and what a snap fills at most): asked for more,
//! the window kept the visible height, so two such heights in a row left
//! its frame unchanged and no resize to read (CI's displays are shorter
//! than a desktop's).
fn main() {
    #[cfg(target_os = "macos")]
    {
        // Taller, as tall as the screen allows (a snap filling it), then
        // shorter than at first (a 400-point window).
        let readings = zgx_desktop::debug_titlebar_mid_resize(|visible| {
            assert!(
                visible >= 500.,
                "the screen's visible frame is {visible} tall: no room to grow a 400-point window"
            );
            vec![(visible + 400.) / 2., visible, 300.]
        });
        assert_eq!(readings.len(), 3, "no window (not the main thread?)");
        for resize in readings {
            let height = resize.asked;
            assert_eq!(
                resize.took, height,
                "the window wasn't {height} tall: {resize:?}"
            );
            let Some(during) = resize.during else {
                panic!("{height}: no resize reported: {resize:?}");
            };
            let after = resize.after;
            let (Some(area), Some(lights)) = after else {
                panic!("{height}: no title bar once AppKit was done: {after:?}");
            };
            // AppKit's own places for them, at the window's top.
            assert!(
                lights.y >= 0. && lights.y < area.height,
                "{height}: {lights:?} in {area:?}"
            );
            assert!(area.height <= 60., "{height}: {area:?}");
            assert_eq!(
                during, after,
                "{height} tall: as winit reported the resize, the title bar read \
                 {during:?}, not where it stands ({after:?})"
            );
        }
        println!("titlebar_resize: an inset title bar reads alike during a resize and after it");
    }
}
