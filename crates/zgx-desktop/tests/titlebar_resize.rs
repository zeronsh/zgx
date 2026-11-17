//! macOS, on the main thread (`fn main`): an inset title bar's buttons
//! (`WindowInfo::controls`, which a title bar laid out around the traffic
//! lights follows) and its area as zgx reads them when winit reports a
//! resize. A window snapped by a window manager (Raycast, Rectangle: its
//! frame set through Accessibility, AXPosition then AXSize) has its content
//! view resized before AppKit moves the title bar to the new top, and winit
//! reports the resize from the content view: read against it, the lights
//! stood the height's change down the window (Zeron's title bar row drew
//! over the sidebar, 900 points down) and the bar was that change tall.
fn main() {
    #[cfg(target_os = "macos")]
    {
        // Taller, much taller, then shorter than at first.
        let heights = [700., 1770., 300.];
        let readings = zgx_desktop::debug_titlebar_mid_resize(&heights);
        assert_eq!(
            readings.len(),
            heights.len(),
            "no window (not the main thread?)"
        );
        for (height, [during, after]) in heights.iter().zip(readings) {
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
