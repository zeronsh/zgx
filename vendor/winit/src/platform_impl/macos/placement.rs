//! zgx patch: where a window asked for at a position is made, as plain
//! arithmetic over the screens' frames (no AppKit), so every arrangement is
//! tested without real screens (zgx-desktop's `tests/screen_placement.rs`
//! includes this file).
//!
//! winit's positions are in a desktop space whose origin is the primary
//! screen's top left, y down; a physical position is a screen's points times
//! that screen's scale (as `MonitorHandle::position` gives them). AppKit's
//! frames have their origin at the primary screen's bottom left, y up.

/// A screen as AppKit has it: `frame` in Cocoa coordinates and its backing
/// scale. The primary screen (`NSScreen::screens()[0]`, the one the menu bar
/// was first on) has its origin at (0, 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Screen {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

impl Screen {
    /// Its top left in winit's desktop space, in points.
    fn top_left(&self, primary_height: f64) -> (f64, f64) {
        (self.x, primary_height - (self.y + self.height))
    }

    /// Whether `(x, y)`, in points of winit's desktop space, is on it.
    fn holds(&self, primary_height: f64, (x, y): (f64, f64)) -> bool {
        let (left, top) = self.top_left(primary_height);
        (left..left + self.width).contains(&x) && (top..top + self.height).contains(&y)
    }

    /// How far `(x, y)` (points, desktop space) is from it, squared.
    fn distance(&self, primary_height: f64, (x, y): (f64, f64)) -> f64 {
        let (left, top) = self.top_left(primary_height);
        let dx = (left - x).max(x - (left + self.width)).max(0.0);
        let dy = (top - y).max(y - (top + self.height)).max(0.0);
        dx * dx + dy * dy
    }
}

/// A position or size as the window's attributes give it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Units {
    Physical(f64, f64),
    Logical(f64, f64),
}

/// Where to make a window: `screen` (an index into the screens given) and
/// its content rect relative to that screen's origin, in Cocoa coordinates,
/// as `initWithContentRect:styleMask:backing:defer:screen:` reads it; and
/// the scale its size was read at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    pub screen: usize,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

/// Places a window whose content's top left is asked for at `position`,
/// its content `size` (points at the chosen screen's scale when physical):
/// on the screen that holds that top left (see below for physical positions
/// two screens hold), or the nearest one when none does. `None` without
/// screens.
pub(crate) fn place(screens: &[Screen], position: Units, size: Units) -> Option<Placement> {
    let primary_height = screens.first()?.height;
    // A physical position is on the screen whose physical rect holds it:
    // each screen's pixels start at its points' origin times its own scale.
    // Screens of different scales overlap in that space (a 1x screen right
    // of a 2x one starts inside the 2x one's pixels), and there the screen
    // whose origin is nearest above and left of it takes it: a screen's own
    // `MonitorHandle::position`, and points just in from it, are on that
    // screen.
    let (index, point) = match position {
        Units::Logical(x, y) => (nearest(screens, primary_height, (x, y)), (x, y)),
        Units::Physical(x, y) => {
            let offset = |screen: &Screen| {
                let (left, top) = screen.top_left(primary_height);
                (x - left * screen.scale) + (y - top * screen.scale)
            };
            let on = (0..screens.len())
                .filter(|&i| {
                    let screen = &screens[i];
                    screen.holds(primary_height, (x / screen.scale, y / screen.scale))
                })
                .min_by(|&a, &b| offset(&screens[a]).total_cmp(&offset(&screens[b])));
            let index = on.unwrap_or_else(|| {
                // Off every screen: the one nearest it in points, read at the
                // primary's scale.
                let scale = screens[0].scale;
                nearest(screens, primary_height, (x / scale, y / scale))
            });
            let scale = screens[index].scale;
            (index, (x / scale, y / scale))
        }
    };
    let screen = screens[index];
    let (width, height) = match size {
        Units::Logical(width, height) => (width, height),
        Units::Physical(width, height) => (width / screen.scale, height / screen.scale),
    };
    // The content's bottom left in Cocoa coordinates, then relative to the
    // screen's origin.
    let bottom = primary_height - point.1 - height;
    Some(Placement {
        screen: index,
        x: point.0 - screen.x,
        y: bottom - screen.y,
        width,
        height,
        scale: screen.scale,
    })
}

/// The screen holding `point` (points, desktop space), or the one nearest it.
fn nearest(screens: &[Screen], primary_height: f64, point: (f64, f64)) -> usize {
    screens
        .iter()
        .position(|screen| screen.holds(primary_height, point))
        .unwrap_or_else(|| {
            (0..screens.len())
                .min_by(|&a, &b| {
                    let a = screens[a].distance(primary_height, point);
                    let b = screens[b].distance(primary_height, point);
                    a.total_cmp(&b)
                })
                .unwrap_or(0)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(x: f64, y: f64, width: f64, height: f64, scale: f64) -> Screen {
        Screen {
            x,
            y,
            width,
            height,
            scale,
        }
    }

    /// Asks for a 600×400 px window a quarter into each screen (physical,
    /// from the screen's top left in desktop pixels, as the real-window test
    /// does) and checks it is placed on that screen, there.
    fn quarter_into_each(screens: &[Screen]) {
        let primary_height = screens[0].height;
        for (index, on) in screens.iter().enumerate() {
            let (left, top) = on.top_left(primary_height);
            let (px, py) = (left * on.scale, top * on.scale);
            let asked = (
                px + on.width * on.scale / 4.0,
                py + on.height * on.scale / 4.0,
            );
            let got = place(
                screens,
                Units::Physical(asked.0, asked.1),
                Units::Physical(600.0, 400.0),
            )
            .expect("a placement");
            let (width, height) = (600.0 / on.scale, 400.0 / on.scale);
            let want = Placement {
                screen: index,
                x: on.width / 4.0,
                // From the screen's bottom: its height less a quarter less the window.
                y: on.height - on.height / 4.0 - height,
                width,
                height,
                scale: on.scale,
            };
            assert_eq!(got, want, "screen {index} of {screens:?}");
            // Back to winit's desktop pixels: where it was asked to be.
            let top_left_y = primary_height - (on.y + got.y + got.height);
            assert_eq!(((on.x + got.x) * on.scale, top_left_y * on.scale), asked);
        }
    }

    #[test]
    fn one_screen() {
        quarter_into_each(&[screen(0.0, 0.0, 1512.0, 982.0, 2.0)]);
    }

    #[test]
    fn same_screens_left_of_the_primary() {
        // Wing's M5: two 6400×3600 px screens at 2x, the second to the left.
        let screens = [
            screen(0.0, 0.0, 3200.0, 1800.0, 2.0),
            screen(-3200.0, 0.0, 3200.0, 1800.0, 2.0),
        ];
        quarter_into_each(&screens);
        // The window the real test asks for on the left screen.
        let got = place(
            &screens,
            Units::Physical(-4800.0, 900.0),
            Units::Physical(600.0, 400.0),
        );
        assert_eq!(
            got,
            Some(Placement {
                screen: 1,
                x: 800.0,
                y: 1150.0,
                width: 300.0,
                height: 200.0,
                scale: 2.0
            })
        );
    }

    #[test]
    fn screens_right_above_and_below_of_other_heights_and_scales() {
        let primary = screen(0.0, 0.0, 1512.0, 982.0, 2.0);
        for screens in [
            // Right, taller, 1x, bottoms aligned.
            [primary, screen(1512.0, 0.0, 2560.0, 1440.0, 1.0)],
            // Right, shorter, its top level with the primary's.
            [primary, screen(1512.0, 982.0 - 720.0, 1280.0, 720.0, 1.0)],
            // Left, taller, 1x, hanging below the primary.
            [primary, screen(-2560.0, -900.0, 2560.0, 1440.0, 1.0)],
            // Above (Cocoa y up), offset right, 1.5x.
            [primary, screen(300.0, 982.0, 1920.0, 1080.0, 1.5)],
            // Below, offset left, 2x.
            [primary, screen(-400.0, -1117.0, 1728.0, 1117.0, 2.0)],
            // Right and 3x.
            [primary, screen(1512.0, 100.0, 1000.0, 600.0, 3.0)],
        ] {
            quarter_into_each(&screens);
        }
        // Three: left, primary, right above.
        quarter_into_each(&[
            primary,
            screen(-1920.0, 0.0, 1920.0, 1080.0, 1.0),
            screen(1512.0, 982.0, 2560.0, 1440.0, 2.0),
        ]);
    }

    #[test]
    fn logical_positions_and_sizes_are_points() {
        let screens = [
            screen(0.0, 0.0, 1512.0, 982.0, 2.0),
            screen(1512.0, 0.0, 2560.0, 1440.0, 1.0),
        ];
        // 100 points into the right screen, 200 down from the primary's top.
        let got = place(
            &screens,
            Units::Logical(1612.0, 200.0),
            Units::Logical(300.0, 200.0),
        );
        assert_eq!(
            got,
            Some(Placement {
                screen: 1,
                x: 100.0,
                y: 982.0 - 200.0 - 200.0,
                width: 300.0,
                height: 200.0,
                scale: 1.0
            })
        );
    }

    #[test]
    fn the_seam_between_two_screens_belongs_to_the_right_one() {
        let screens = [
            screen(0.0, 0.0, 1000.0, 800.0, 1.0),
            screen(-1000.0, 0.0, 1000.0, 800.0, 1.0),
        ];
        let got = place(
            &screens,
            Units::Logical(0.0, 10.0),
            Units::Logical(10.0, 10.0),
        )
        .unwrap();
        assert_eq!(got.screen, 0);
        let got = place(
            &screens,
            Units::Logical(-0.5, 10.0),
            Units::Logical(10.0, 10.0),
        )
        .unwrap();
        assert_eq!(got.screen, 1);
    }

    #[test]
    fn off_every_screen_the_nearest_one_takes_it() {
        let screens = [
            screen(0.0, 0.0, 1000.0, 800.0, 2.0),
            screen(-1000.0, 0.0, 1000.0, 800.0, 1.0),
        ];
        // Far left of both, level with them: the left screen, at its scale.
        let got = place(
            &screens,
            Units::Physical(-5000.0, 100.0),
            Units::Physical(100.0, 100.0),
        )
        .unwrap();
        assert_eq!(got.screen, 1);
        assert_eq!(
            (got.x, got.width, got.scale),
            (-5000.0 + 1000.0, 100.0, 1.0)
        );
        // Above the primary: the primary, a negative distance down.
        let got = place(
            &screens,
            Units::Logical(500.0, -50.0),
            Units::Logical(100.0, 100.0),
        )
        .unwrap();
        assert_eq!((got.screen, got.y), (0, 800.0 + 50.0 - 100.0));
        assert!(place(&[], Units::Logical(0.0, 0.0), Units::Logical(1.0, 1.0)).is_none());
    }
}
