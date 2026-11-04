//! Window chrome from `WindowOptions`: the title bar's style and a frosted
//! glass behind a translucent window. Applied when the window is made, on
//! macOS and Windows; on Linux zgx draws an undecorated window's buttons
//! and resizes it from its edges, and asks the compositor to blur what is
//! behind a window with glass (`ext-background-effect-v1`, or KWin's blur
//! protocol on Wayland). How much it blurs is the compositor's: Hyprland's
//! is its `decoration:blur` settings, and only when blur is enabled there.

/// How a window's title bar looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Titlebar {
    /// The platform's title bar.
    #[default]
    Standard,
    /// Transparent, without the title: content runs under it, the window
    /// buttons (the traffic lights) inset over it in a 40-point strip, as
    /// Electron's `hiddenInset`; a zoomed window's strip is 40 of its zoomed
    /// points, the traffic lights centred in it (`zoom_buttons`). Mark the strip `View::window_drag` so it
    /// moves the window. On Windows, zgx draws Windows 11's caption buttons
    /// over the content's top right (46 by 32 points of the display each,
    /// whatever the window's zoom, or as tall as `WindowOptions::caption_height`;
    /// a window the person can't resize has no Maximize) and Windows takes
    /// them for its own: their tooltips, and snap layouts on maximize. On
    /// Linux the window is undecorated, as a GTK header bar's, and zgx
    /// draws libadwaita's window buttons over the strip, in the order the
    /// desktop asks.
    Inset,
    /// None: content fills the window and the window buttons are hidden.
    /// The window keeps its rounded corners and shadow; on Windows and Linux
    /// its edges resize it (5 points in from them).
    Hidden,
}

/// A blurred desktop behind a translucent window (macOS's behind-window
/// `NSVisualEffectView`, cut to the window's rounded corners; on Windows 11
/// the system backdrop nearest its material, Acrylic or Mica; on Linux the
/// compositor's blur, asked for where it has a protocol for it, with no
/// material, tint or saturation of zgx's). The window is transparent where
/// zgx paints nothing (or translucent colour).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glass {
    /// The blur's material.
    pub material: GlassMaterial,
    /// Only the blur: no tint from the desktop, no saturation boost, a
    /// 60-point gaussian, as gpui's `BlurredView` (for apps laying their own
    /// tint over it).
    pub plain: bool,
}
impl Glass {
    /// Glass of `material`, tinted and saturated as the platform's is.
    pub fn new(material: GlassMaterial) -> Self {
        Self {
            material,
            plain: false,
        }
    }
    /// The same glass, untinted and unsaturated (see `Glass::plain`).
    pub fn plain(mut self) -> Self {
        self.plain = true;
        self
    }
}
impl Default for Glass {
    fn default() -> Self {
        Self::new(GlassMaterial::Hud)
    }
}
/// The glass's material (`NSVisualEffectMaterial`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlassMaterial {
    /// A dark, saturated heads-up-display glass.
    #[default]
    Hud,
    /// The colourless material under a window's own background.
    UnderWindowBackground,
    /// The material behind a page's content.
    UnderPageBackground,
    /// A window's background.
    WindowBackground,
    /// The background of a window's content.
    ContentBackground,
    /// A title bar's.
    Titlebar,
    /// A header view's, as a table's column headers.
    HeaderView,
    /// A sidebar's.
    Sidebar,
    /// A menu's.
    Menu,
    /// A popover's.
    Popover,
    /// A sheet's.
    Sheet,
    /// A tooltip's.
    ToolTip,
    /// A fullscreen window's controls.
    FullScreenUi,
    /// The selection's highlight.
    Selection,
}
impl GlassMaterial {
    /// Its `NSVisualEffectMaterial` value.
    pub fn appkit_value(self) -> isize {
        match self {
            Self::Titlebar => 3,
            Self::Selection => 4,
            Self::Menu => 5,
            Self::Popover => 6,
            Self::Sidebar => 7,
            Self::HeaderView => 10,
            Self::Sheet => 11,
            Self::WindowBackground => 12,
            Self::Hud => 13,
            Self::FullScreenUi => 15,
            Self::ToolTip => 17,
            Self::ContentBackground => 18,
            Self::UnderWindowBackground => 21,
            Self::UnderPageBackground => 22,
        }
    }
}

/// Style `window` as `titlebar` and `glass` ask: its corner radius in points
/// (content inset from the window's edge keeps its corners concentric with
/// it), 0 where the platform does not round windows or zgx cannot tell.
pub(crate) fn apply(
    window: &winit::window::Window,
    titlebar: Titlebar,
    glass: Option<Glass>,
    caption_height: Option<f64>,
) -> f64 {
    #[cfg(target_os = "macos")]
    {
        let _ = caption_height;
        mac::apply(window, titlebar, glass)
    }
    #[cfg(windows)]
    {
        win::apply(window, titlebar, glass, caption_height)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (window, titlebar, glass, caption_height);
        0.
    }
}

/// Give `window` `glass` in place of the glass it has (`None`: none), as
/// `WindowHandle::set_glass` asks. Its content shows the glass only where
/// it is transparent: a window made opaque (neither `transparent` nor with
/// glass) shows none.
pub(crate) fn set_glass(window: &winit::window::Window, glass: Option<Glass>) {
    #[cfg(target_os = "macos")]
    mac::set_glass(window, glass);
    #[cfg(windows)]
    win::set_glass(window, glass);
    // The compositor's blur follows the glass, on and off (Wayland; a no-op
    // on X11), from the surface's next commit: the redraw the caller asks.
    #[cfg(target_os = "linux")]
    window.set_blur(glass.is_some());
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    let _ = (window, glass);
}

/// Place an inset title bar's window buttons for a window zoomed to `zoom`
/// (`Ui::zoom`): on macOS the traffic lights, which AppKit places in points
/// the zoom does not touch, move to where the zoomed content's strip has
/// them (centred in 40 × zoom points, as far in as the zoom makes AppKit's
/// inset), so a title bar laid out around them lines up at any zoom. Back at
/// 100%, AppKit's places. AppKit lays them out again as the window resizes:
/// the host calls this after.
pub(crate) fn zoom_buttons(window: &winit::window::Window, titlebar: Titlebar, zoom: f64) {
    #[cfg(target_os = "macos")]
    if titlebar == Titlebar::Inset {
        mac::zoom_buttons(window, zoom);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (window, titlebar, zoom);
}

/// The title bar's free area and the window's buttons in it, in points
/// (`WindowInfo::titlebar_area`, `controls`): `None` for a standard title
/// bar and in fullscreen.
pub(crate) fn titlebar_area(
    window: &winit::window::Window,
    titlebar: Titlebar,
) -> (Option<zgx::scene::Rect>, Option<zgx::scene::Rect>) {
    if titlebar == Titlebar::Standard || window.fullscreen().is_some() {
        return (None, None);
    }
    let size = window.inner_size().to_logical::<f32>(window.scale_factor());
    #[cfg(windows)]
    {
        let _ = size;
        win::titlebar_area(window, titlebar)
    }
    #[cfg(target_os = "linux")]
    {
        linux::titlebar_area(size.width, titlebar)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = size;
        mac::titlebar_area(window)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = size;
        (None, None)
    }
}

#[cfg(windows)]
pub(crate) use win::{CaptionHost, fit_work_area, menu_theme};

#[cfg(windows)]
mod win {
    //! Windows title bars. `Titlebar::Hidden` is winit's frameless window
    //! (its client the whole window, with the system's shadow) with what
    //! the frame gave back: rounded corners on Windows 11 and resize bands
    //! along its edges. `Titlebar::Inset` adds the caption buttons, drawn
    //! by zgx over the content's top right (`CaptionHost`) as Windows 11
    //! draws its own, and reported to Windows as its buttons
    //! (`WM_NCHITTEST`), so it shows the maximize button's snap layouts and
    //! the buttons' tooltips. Windows' own handling of a press on them draws
    //! classic buttons into the window, so the subclass handles them itself.
    //! Where they are and what shows them is `crate::windows_caption`'s.
    use super::{Glass, GlassMaterial, Titlebar};
    use crate::windows_caption::{CaptionButton, CaptionLook, Geometry, Hit};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
        sync::Arc,
    };
    use windows::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Dwm::{
                DWM_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMSBT_TABBEDWINDOW,
                DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
                DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmExtendFrameIntoClientArea,
                DwmSetWindowAttribute,
            },
            Gdi::ScreenToClient,
        },
        UI::{
            Controls::MARGINS,
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent},
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                GetClientRect, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCLOSE, HTLEFT, HTMAXBUTTON,
                HTMINBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed, PostMessageW,
                SC_CLOSE, SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE, SendMessageW, WM_MOUSEMOVE,
                WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
                WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_SETTINGCHANGE, WM_SYSCOLORCHANGE,
                WM_SYSCOMMAND, WM_THEMECHANGED,
            },
        },
    };
    use winit::{
        raw_window_handle::{HasWindowHandle, RawWindowHandle},
        window::Window,
    };
    use zgx::widgets::Ui;

    /// Windows 11's window corner radius, in logical pixels.
    const CORNER_RADIUS: f64 = 8.;

    /// What `WM_NCHITTEST` answers for `button`, as Windows' own buttons do.
    fn button_hit(button: CaptionButton) -> u32 {
        match button {
            CaptionButton::Minimize => HTMINBUTTON,
            CaptionButton::Maximize => HTMAXBUTTON,
            CaptionButton::Close => HTCLOSE,
        }
    }
    fn button_of(hit: usize) -> Option<CaptionButton> {
        [
            CaptionButton::Minimize,
            CaptionButton::Maximize,
            CaptionButton::Close,
        ]
        .into_iter()
        .find(|button| button_hit(*button) as usize == hit)
    }
    /// What `WM_NCHITTEST` answers for `hit`.
    fn hit_code(hit: Hit) -> u32 {
        match hit {
            Hit::Button(button) => button_hit(button),
            Hit::Left => HTLEFT,
            Hit::Right => HTRIGHT,
            Hit::Top => HTTOP,
            Hit::Bottom => HTBOTTOM,
            Hit::TopLeft => HTTOPLEFT,
            Hit::TopRight => HTTOPRIGHT,
            Hit::BottomLeft => HTBOTTOMLEFT,
            Hit::BottomRight => HTBOTTOMRIGHT,
        }
    }

    /// What the window procedure knows of the caption, shared with its host.
    struct Caption {
        /// The window has no frame (Inset, Hidden): its edges resize it.
        frameless: bool,
        buttons: bool,
        /// The window runs right to left (the user's language,
        /// `ZGX_DIRECTION`): its buttons are at its top left.
        rtl: bool,
        /// The buttons show: not in fullscreen, where a window has none.
        shown: Cell<bool>,
        /// `WindowOptions::caption_height`.
        height: Option<f64>,
        /// The window's zoom, and whether the person can resize it, as of
        /// its last frame: what the geometry hit-tests by.
        zoom: Cell<f64>,
        resizable: Cell<bool>,
        /// Its smallest and largest sizes are the same: a window of fixed
        /// size, though window managers can still move it (`CaptionHost::
        /// set_fixed_size`).
        fixed_size: Cell<bool>,
        /// The system's settings or colours changed (high contrast, the
        /// accent, animations) since the host last read them.
        settings_changed: Cell<bool>,
        hovered: Cell<Option<CaptionButton>>,
        pressed: Cell<Option<CaptionButton>>,
        tracking: Cell<bool>,
        /// Asks the host for a frame when the buttons' look changed.
        wake: RefCell<Option<Box<dyn Fn()>>>,
    }
    impl Caption {
        /// Where its buttons are, at display `scale`: the same for what
        /// draws them, hit-tests them and reports them.
        fn geometry(&self, scale: f64) -> Geometry {
            Geometry::new(
                scale,
                self.zoom.get(),
                self.height,
                self.resizable.get() && !self.fixed_size.get(),
                self.rtl,
            )
        }
        fn set(&self, hovered: Option<CaptionButton>, pressed: Option<CaptionButton>) {
            let hover_changed = self.hovered.replace(hovered) != hovered;
            let press_changed = self.pressed.replace(pressed) != pressed;
            if (hover_changed || press_changed)
                && let Some(wake) = &*self.wake.borrow()
            {
                wake();
            }
        }
    }

    fn hwnd(window: &Window) -> Option<HWND> {
        match window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut _)),
            _ => None,
        }
    }

    /// The system backdrop nearest a glass material: Acrylic (a blur, as
    /// macOS's materials are) for the transient and heads-up ones, Mica (the
    /// wallpaper's tint) under a window's or page's background, Mica Alt
    /// under title bars and headers.
    fn backdrop(material: GlassMaterial) -> DWM_SYSTEMBACKDROP_TYPE {
        match material {
            GlassMaterial::UnderWindowBackground
            | GlassMaterial::UnderPageBackground
            | GlassMaterial::WindowBackground
            | GlassMaterial::ContentBackground
            | GlassMaterial::Sidebar => DWMSBT_MAINWINDOW,
            GlassMaterial::Titlebar | GlassMaterial::HeaderView => DWMSBT_TABBEDWINDOW,
            GlassMaterial::Hud
            | GlassMaterial::Menu
            | GlassMaterial::Popover
            | GlassMaterial::ToolTip
            | GlassMaterial::Sheet
            | GlassMaterial::FullScreenUi
            | GlassMaterial::Selection => DWMSBT_TRANSIENTWINDOW,
        }
    }

    /// The window's backdrop replaced (`super::set_glass`): `None` takes the
    /// system's backdrop and the frame extended under the client away.
    pub(super) fn set_glass(window: &Window, glass: Option<Glass>) {
        let Some(hwnd) = hwnd(window) else {
            return;
        };
        match glass {
            Some(glass) => frost(
                hwnd,
                glass,
                window.theme() != Some(winit::window::Theme::Light),
            ),
            None => {
                let margins = MARGINS::default();
                let kind = DWMSBT_NONE;
                // SAFETY: our own window; the attribute is given its size.
                unsafe {
                    let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
                    let _ = DwmSetWindowAttribute(
                        hwnd,
                        DWMWA_SYSTEMBACKDROP_TYPE,
                        &kind as *const _ as *const _,
                        size_of_val(&kind) as u32,
                    );
                }
            }
        }
    }

    /// Put the system's backdrop behind the window: the frame, and its
    /// material, extended under the whole client, which shows it where zgx
    /// paints nothing. Windows 11 (22H2) only; elsewhere the window is
    /// transparent alone. `Glass::plain` has no Windows equivalent.
    fn frost(hwnd: HWND, glass: Glass, dark: bool) {
        // From the bottom, over the whole client: extended from the top, the
        // frame brings Windows' own caption buttons into the client, under
        // zgx's (or under nothing, where it paints none).
        let margins = MARGINS {
            cxLeftWidth: 0,
            cxRightWidth: 0,
            cyTopHeight: 0,
            cyBottomHeight: i32::from(u16::MAX),
        };
        let kind = backdrop(glass.material);
        let dark = i32::from(dark);
        // SAFETY: our own window; each attribute is given its own size.
        unsafe {
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const _ as *const _,
                size_of_val(&dark) as u32,
            );
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                &kind as *const _ as *const _,
                size_of_val(&kind) as u32,
            );
        }
    }

    /// The executable's own icon (its first icon group, the one Explorer
    /// shows) on the window's title bar, taskbar button and Alt+Tab: winit's
    /// windows have none, and Windows drew a blank page. An executable
    /// without one keeps that. Sized for the window's DPI.
    fn icon(hwnd: HWND) {
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        // SAFETY: this executable's own module.
        if let Ok(module) = unsafe { GetModuleHandleW(None) } {
            set_icons(hwnd, module);
        }
    }

    /// `module`'s first icon group as `hwnd`'s big and small icons, sized for
    /// its DPI. Windows destroys neither the icons a window was given
    /// (`WM_SETICON`) nor those a later one replaces: one replaced is
    /// destroyed here, and the window's last at `release_icons`.
    fn set_icons(hwnd: HWND, module: windows::Win32::Foundation::HMODULE) {
        use windows::{
            Win32::{
                Foundation::{HMODULE, LPARAM, WPARAM},
                System::LibraryLoader::EnumResourceNamesW,
                UI::{
                    HiDpi::GetSystemMetricsForDpi,
                    WindowsAndMessaging::{
                        DestroyIcon, HICON, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR,
                        LoadImageW, RT_GROUP_ICON, SM_CXICON, SM_CXSMICON, SendMessageW,
                        WM_SETICON,
                    },
                },
            },
            core::{BOOL, PCWSTR},
        };
        /// The first icon group's name: an integer ID, or a string copied out.
        enum Name {
            Id(usize),
            Text(Vec<u16>),
        }
        /// # Safety
        ///
        /// Called by `EnumResourceNamesW` alone, with `found` the
        /// `Option<Name>` of the call below.
        unsafe extern "system" fn first(_: HMODULE, _: PCWSTR, name: PCWSTR, found: isize) -> BOOL {
            // SAFETY: `found` is the `Option<Name>` below, borrowed by
            // nothing else while the enumeration runs; a name that is not an
            // integer ID is a NUL-terminated string, valid during the call.
            unsafe {
                let found = &mut *(found as *mut Option<Name>);
                *found = Some(if (name.0 as usize) >> 16 == 0 {
                    Name::Id(name.0 as usize)
                } else {
                    let mut text = name.as_wide().to_vec();
                    text.push(0);
                    Name::Text(text)
                });
            }
            false.into()
        }
        // SAFETY: `module`'s resources; the callback writes `found` only
        // while the enumeration runs, and `name` borrows `found`'s string (or
        // is an integer ID) for the loads after it. An icon the window gives
        // back for a new one is ours, and no longer shown.
        unsafe {
            let mut found: Option<Name> = None;
            let _ = EnumResourceNamesW(
                Some(module),
                RT_GROUP_ICON,
                Some(first),
                &mut found as *mut _ as isize,
            );
            let name = match &found {
                Some(Name::Id(id)) => PCWSTR(*id as *const u16),
                Some(Name::Text(text)) => PCWSTR(text.as_ptr()),
                None => return,
            };
            let dpi = GetDpiForWindow(hwnd);
            for (size, which) in [(SM_CXICON, ICON_BIG), (SM_CXSMICON, ICON_SMALL)] {
                let side = GetSystemMetricsForDpi(size, dpi);
                if let Ok(icon) = LoadImageW(
                    Some(module.into()),
                    name,
                    IMAGE_ICON,
                    side,
                    side,
                    LR_DEFAULTCOLOR,
                ) {
                    let replaced = SendMessageW(
                        hwnd,
                        WM_SETICON,
                        Some(WPARAM(which as usize)),
                        Some(LPARAM(icon.0 as isize)),
                    );
                    if replaced.0 != 0 {
                        let _ = DestroyIcon(HICON(replaced.0 as *mut _));
                    }
                }
            }
        }
    }

    /// Take `hwnd`'s icons (`set_icons`) back and destroy them, as it goes:
    /// each held its bitmaps, GDI objects, for the life of the process.
    fn release_icons(hwnd: HWND) {
        use windows::Win32::{
            Foundation::WPARAM,
            UI::WindowsAndMessaging::{
                DestroyIcon, HICON, ICON_BIG, ICON_SMALL, SendMessageW, WM_SETICON,
            },
        };
        for which in [ICON_BIG, ICON_SMALL] {
            // SAFETY: our own window, which gives back the icon it had: ours,
            // shown nowhere else.
            unsafe {
                let old = SendMessageW(hwnd, WM_SETICON, Some(WPARAM(which as usize)), None);
                if old.0 != 0 {
                    let _ = DestroyIcon(HICON(old.0 as *mut _));
                }
            }
        }
    }

    /// The caption strip of a frameless window, and the caption buttons
    /// `Inset` draws at its right, or its left where windows run right to
    /// left (none for `Hidden`), in points of the display.
    pub(super) fn titlebar_area(
        window: &Window,
        titlebar: Titlebar,
    ) -> (Option<zgx::scene::Rect>, Option<zgx::scene::Rect>) {
        let caption = hwnd(window).and_then(|hwnd| caption_of(hwnd.0 as usize));
        let geometry = Geometry::new(
            window.scale_factor(),
            crate::popup_window::zoom_of(window.id()),
            caption.as_ref().and_then(|caption| caption.height),
            window.is_resizable() && !caption.is_some_and(|caption| caption.fixed_size.get()),
            rtl(),
        );
        let width = window.inner_size().width as i32;
        let (area, controls) = geometry.strip(width, titlebar == Titlebar::Inset);
        (Some(area), controls)
    }

    /// Fit a new window to its monitor's work area (the screen less the
    /// taskbar): a size asked for in points can be more pixels than the
    /// screen has at a high scale (1180 by 760 points is 1770 by 1140
    /// pixels at 150%, past a 1080-pixel screen), and Windows, unlike macOS,
    /// shows it so. It shrinks to fit, though not below `min` (logical), and
    /// moves inside.
    pub(crate) fn fit_work_area(window: &Window, min: Option<(f64, f64)>) {
        use windows::Win32::{
            Graphics::Gdi::{
                GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
            },
            UI::WindowsAndMessaging::{GetWindowRect, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos},
        };
        let Some(hwnd) = hwnd(window) else {
            return;
        };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let mut rect = RECT::default();
        // SAFETY: our own window; each call writes into a local.
        unsafe {
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            if !GetMonitorInfoW(monitor, &mut info).as_bool()
                || GetWindowRect(hwnd, &mut rect).is_err()
            {
                return;
            }
        }
        let work = info.rcWork;
        let scale = scale(hwnd);
        let (min_w, min_h) = min.map_or((0, 0), |(w, h)| {
            ((w * scale).ceil() as i32, (h * scale).ceil() as i32)
        });
        let fitted = fit(
            (
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
            ),
            (
                work.left,
                work.top,
                work.right - work.left,
                work.bottom - work.top,
            ),
            (min_w, min_h),
        );
        if fitted
            != (
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
            )
        {
            let (x, y, width, height) = fitted;
            // SAFETY: our own window, still hidden.
            let _ = unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    x,
                    y,
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
        }
    }

    /// Window rectangle `(x, y, width, height)` fitted inside `work`: no
    /// larger than it (nor smaller than `min`), and moved inside it.
    fn fit(
        (x, y, width, height): (i32, i32, i32, i32),
        (wx, wy, ww, wh): (i32, i32, i32, i32),
        (min_w, min_h): (i32, i32),
    ) -> (i32, i32, i32, i32) {
        let width = width.min(ww).max(min_w);
        let height = height.min(wh).max(min_h);
        let x = x.min(wx + ww - width).max(wx);
        let y = y.min(wy + wh - height).max(wy);
        (x, y, width, height)
    }

    /// Windows' own menu for the window (Alt+Space, a right click on its
    /// title bar) follows the system's dark mode, as Explorer's and
    /// Terminal's do; winit darkens only the title bar. Called as the window
    /// is made and when the mode changes. uxtheme exports this only by
    /// ordinal, its numbers other functions' before Windows 10 1903 (18362).
    /// "Allow dark", not forced: winit reads the system's mode through
    /// uxtheme, and a forced mode would hide its changes.
    pub(crate) fn menu_theme(window: &Window) {
        use std::sync::OnceLock;
        use windows::{
            Wdk::System::SystemServices::RtlGetVersion,
            Win32::System::{
                LibraryLoader::{GetProcAddress, LoadLibraryW},
                SystemInformation::OSVERSIONINFOW,
            },
            core::{BOOL, PCSTR, w},
        };
        type Proc = unsafe extern "system" fn() -> isize;
        type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
        type Refresh = unsafe extern "system" fn();
        type AllowDarkModeForWindow = unsafe extern "system" fn(HWND, BOOL) -> BOOL;
        type Uxtheme = (SetPreferredAppMode, [Refresh; 2], AllowDarkModeForWindow);
        static UXTHEME: OnceLock<Option<Uxtheme>> = OnceLock::new();
        let uxtheme = UXTHEME.get_or_init(|| {
            let mut version = OSVERSIONINFOW {
                dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
                ..Default::default()
            };
            // SAFETY: fills `version`, whose size it is given.
            if unsafe { RtlGetVersion(&mut version) }.is_err()
                || version.dwMajorVersion < 10
                || version.dwBuildNumber < 18362
            {
                return None;
            }
            // SAFETY: the library's reference is never released, so its
            // functions stay loaded for the process. From 1903 (18362) the
            // ordinals are these functions, of these signatures: but that
            // AllowDarkModeForWindow takes and returns a C++ `bool`, which a
            // `BOOL` of 1 passes the same in its register, and its result
            // is never read.
            unsafe {
                let uxtheme = LoadLibraryW(w!("uxtheme.dll")).ok()?;
                let ordinal = |n: usize| GetProcAddress(uxtheme, PCSTR(n as _));
                let refresh = |n| Some(std::mem::transmute::<Proc, Refresh>(ordinal(n)?));
                Some((
                    std::mem::transmute::<Proc, SetPreferredAppMode>(ordinal(135)?),
                    // RefreshImmersiveColorPolicyState, FlushMenuThemes.
                    [refresh(104)?, refresh(136)?],
                    std::mem::transmute::<Proc, AllowDarkModeForWindow>(ordinal(133)?),
                ))
            }
        });
        let (Some((set_mode, refresh, allow)), Some(hwnd)) = (uxtheme, hwnd(window)) else {
            return;
        };
        // SAFETY: our own window; none of these keeps anything of ours. The
        // window opened its menu's theme when created: it opens it again
        // for the new mode on WM_THEMECHANGED.
        unsafe {
            let _ = allow(hwnd, true.into());
            set_mode(1); // Allow dark.
            refresh.iter().for_each(|refresh| refresh());
            SendMessageW(hwnd, WM_THEMECHANGED, None, None);
        }
    }

    pub(super) fn apply(
        window: &Window,
        titlebar: Titlebar,
        glass: Option<Glass>,
        caption_height: Option<f64>,
    ) -> f64 {
        let Some(hwnd) = hwnd(window) else {
            return 0.;
        };
        menu_theme(window);
        icon(hwnd);
        if let Some(glass) = glass {
            frost(
                hwnd,
                glass,
                window.theme() != Some(winit::window::Theme::Light),
            );
        }
        // A frameless window is square unless asked; Windows 10 has no
        // rounded windows and refuses.
        let frameless = titlebar != Titlebar::Standard;
        // SAFETY: sets one documented attribute on our own window.
        let rounded = frameless
            && unsafe {
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &DWMWCP_ROUND as *const _ as *const _,
                    size_of_val(&DWMWCP_ROUND) as u32,
                )
            }
            .is_ok();
        // Every window is subclassed: for the frame's work of a frameless
        // one, and to hear the system's settings change.
        let caption = Rc::new(Caption {
            frameless,
            buttons: titlebar == Titlebar::Inset,
            rtl: rtl(),
            shown: Cell::new(true),
            height: caption_height,
            zoom: Cell::new(crate::popup_window::zoom_of(window.id())),
            resizable: Cell::new(window.is_resizable()),
            fixed_size: Cell::new(false),
            settings_changed: Cell::new(false),
            hovered: Cell::new(None),
            pressed: Cell::new(None),
            tracking: Cell::new(false),
            wake: RefCell::new(None),
        });
        let data = Rc::into_raw(caption.clone());
        // SAFETY: the subclass runs on this (the window's) thread; its
        // reference is released when the window is destroyed (`subclass`
        // on WM_NCDESTROY), or now if it was not installed.
        let installed = unsafe {
            let installed = SetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, data as usize);
            if !installed.as_bool() {
                drop(Rc::from_raw(data));
            }
            installed
        }
        .as_bool();
        if installed {
            CAPTIONS.with(|captions| captions.borrow_mut().push((hwnd.0 as usize, caption)));
        }
        if rounded { CORNER_RADIUS } else { 0. }
    }

    const SUBCLASS_ID: usize = 0x7a67_7569; // "zgx"
    thread_local! {
        /// Each subclassed window's caption, for its host to find.
        static CAPTIONS: RefCell<Vec<(usize, Rc<Caption>)>> = const { RefCell::new(Vec::new()) };
    }

    /// The caption of the window at `hwnd`, if `apply` subclassed it.
    fn caption_of(hwnd: usize) -> Option<Rc<Caption>> {
        CAPTIONS.with(|captions| {
            captions
                .borrow()
                .iter()
                .find(|(window, _)| *window == hwnd)
                .map(|(_, caption)| caption.clone())
        })
    }

    fn scale(hwnd: HWND) -> f64 {
        // SAFETY: reads our own window's DPI.
        f64::from(unsafe { GetDpiForWindow(hwnd) }.max(96)) / 96.
    }

    /// What lies at screen point (`x`, `y`) of the window's client, if not
    /// the content: a resize band or a caption button.
    fn hit_test(hwnd: HWND, caption: &Caption, x: i32, y: i32) -> Option<u32> {
        let mut point = POINT { x, y };
        let mut client = RECT::default();
        // SAFETY: our own window; both write into locals.
        unsafe {
            if !ScreenToClient(hwnd, &mut point).as_bool()
                || GetClientRect(hwnd, &mut client).is_err()
            {
                return None;
            }
        }
        // SAFETY: reads our own window's state.
        let maximized = unsafe { IsZoomed(hwnd) }.as_bool();
        caption
            .geometry(scale(hwnd))
            .hit(
                (point.x, point.y),
                (client.right, client.bottom),
                maximized,
                caption.buttons && caption.shown.get(),
            )
            .map(hit_code)
    }
    /// Whether windows run right to left (the user's language,
    /// `ZGX_DIRECTION`), as their title bars then do.
    fn rtl() -> bool {
        crate::system_colors::locale_direction() == zgx::scene::Direction::Rtl
    }

    /// # Safety
    ///
    /// Called by comctl32 alone, as the subclass `apply` installs: `data` is
    /// the `Rc<Caption>` it leaked for it.
    unsafe extern "system" fn subclass(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        data: usize,
    ) -> LRESULT {
        // SAFETY: `data` is the `Rc` `apply` leaked for this subclass, alive
        // until `WM_NCDESTROY` below takes it back (no other message of the
        // window comes after that one, and nothing here destroys it).
        let caption = unsafe { &*(data as *const Caption) };
        let point = || {
            (
                (lparam.0 & 0xffff) as i16 as i32,
                ((lparam.0 >> 16) & 0xffff) as i16 as i32,
            )
        };
        match message {
            // High contrast on or off, the accent, animations, the colours.
            WM_SETTINGCHANGE | WM_SYSCOLORCHANGE => {
                caption.settings_changed.set(true);
                if let Some(wake) = &*caption.wake.borrow() {
                    wake();
                }
            }
            WM_NCHITTEST if caption.frameless => {
                let (x, y) = point();
                if let Some(hit) = hit_test(hwnd, caption, x, y) {
                    return LRESULT(hit as isize);
                }
            }
            // Every window's, buttons or not.
            WM_NCDESTROY => {
                release_icons(hwnd);
                // SAFETY: removes this subclass and takes back its reference.
                unsafe {
                    let _ = RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID);
                    drop(Rc::from_raw(data as *const Caption));
                }
                CAPTIONS.with(|captions| {
                    captions
                        .borrow_mut()
                        .retain(|(window, _)| *window != hwnd.0 as usize)
                });
            }
            // A standard frame's own buttons are the frame's.
            _ if !caption.buttons => {}
            WM_NCMOUSEMOVE => {
                if !caption.tracking.replace(true) {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    // SAFETY: tracks our own window.
                    let _ = unsafe { TrackMouseEvent(&mut track) };
                }
                let hovered = button_of(wparam.0);
                let pressed = caption.pressed.get().filter(|_| hovered.is_some());
                caption.set(hovered, pressed);
            }
            WM_NCMOUSELEAVE => {
                caption.tracking.set(false);
                caption.set(None, None);
            }
            WM_MOUSEMOVE => caption.set(None, None),
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
                if let Some(button) = button_of(wparam.0) {
                    caption.set(Some(button), Some(button));
                    return LRESULT(0);
                }
            }
            WM_NCLBUTTONUP => {
                if let Some(button) = button_of(wparam.0) {
                    let pressed = caption.pressed.get() == Some(button);
                    caption.set(Some(button), None);
                    if pressed {
                        let command = match button {
                            CaptionButton::Minimize => SC_MINIMIZE,
                            // SAFETY: reads our own window's state.
                            CaptionButton::Maximize if unsafe { IsZoomed(hwnd) }.as_bool() => {
                                SC_RESTORE
                            }
                            CaptionButton::Maximize => SC_MAXIMIZE,
                            CaptionButton::Close => SC_CLOSE,
                        };
                        // The system command animates and asks winit, as
                        // the frame's own buttons do.
                        // SAFETY: posts to our own window.
                        let _ = unsafe {
                            PostMessageW(
                                Some(hwnd),
                                WM_SYSCOMMAND,
                                WPARAM(command as usize),
                                LPARAM(0),
                            )
                        };
                    }
                    return LRESULT(0);
                }
            }
            _ => {}
        }
        // SAFETY: forwards to the next procedure in the chain (winit's).
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }

    /// A window's side of its procedure: the caption buttons of a window
    /// with `Titlebar::Inset`, drawn over its content (hovered, pressed, the
    /// window's state and appearance follow the window procedure, a frame
    /// after they change), and the system's settings read again when they
    /// change.
    pub(crate) struct CaptionHost {
        caption: Rc<Caption>,
        window: std::sync::Weak<Window>,
        look: CaptionLook,
        view: Option<zgx::compose::ViewHandle>,
    }
    impl CaptionHost {
        /// For a window `apply` subclassed; `wake` asks for a frame.
        pub(crate) fn new(
            window: &Arc<Window>,
            ui: &Ui,
            wake: impl Fn() + 'static,
        ) -> Option<Self> {
            let caption = caption_of(hwnd(window)?.0 as usize)?;
            *caption.wake.borrow_mut() = Some(Box::new(wake));
            let look = CaptionLook::new(ui, caption.geometry(window.scale_factor()));
            Some(Self {
                caption,
                window: Arc::downgrade(window),
                look,
                view: None,
            })
        }
        /// The window's smallest and largest sizes are the same (or no
        /// longer are): no resize bands and no Maximize, as for one that
        /// isn't resizable.
        pub(crate) fn set_fixed_size(&self, fixed: bool) {
            self.caption.fixed_size.set(fixed);
        }
        /// Before a frame: bring the buttons up to date, and back if the
        /// content was rendered again over them.
        pub(crate) fn sync(&mut self, ui: &mut Ui) {
            if self.caption.settings_changed.take() {
                crate::system_colors::sync(ui);
                crate::scrollers::sync(ui);
            }
            if !self.caption.buttons {
                return;
            }
            if let Some(window) = self.window.upgrade() {
                self.caption.zoom.set(ui.zoom());
                self.caption.resizable.set(window.is_resizable());
                let geometry = self.caption.geometry(window.scale_factor());
                if self.look.geometry.with_untracked(|old| *old != geometry) {
                    self.look.geometry.set(geometry);
                }
                self.look.maximized.set(window.is_maximized());
                self.look.active.set(window.has_focus());
                let high_contrast = ui.theme().system_preferences().increased_contrast;
                if self
                    .look
                    .high_contrast
                    .with_untracked(|old| *old != high_contrast)
                {
                    self.look.high_contrast.set(high_contrast);
                }
                let fullscreen = window.fullscreen().is_some();
                self.look.fullscreen.set(fullscreen);
                self.caption.shown.set(!fullscreen);
            }
            self.look.hovered.set(self.caption.hovered.get());
            self.look.pressed.set(self.caption.pressed.get());
            if !self.view.as_ref().is_some_and(|view| view.is_mounted()) {
                self.view = Some(ui.mount(self.look.view(self.caption.rtl)));
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Windows given icons (twice, as on a DPI change) and destroyed
        /// leave no GDI objects behind; without `release_icons` each left its
        /// icons' bitmaps. user32.dll's icon groups stand in for an
        /// executable's.
        #[test]
        fn windows_given_icons_leave_no_gdi_objects_behind() {
            use windows::Win32::{
                System::{
                    LibraryLoader::{
                        LOAD_LIBRARY_AS_DATAFILE, LOAD_LIBRARY_AS_IMAGE_RESOURCE, LoadLibraryExW,
                    },
                    Threading::{GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources},
                },
                UI::WindowsAndMessaging::{
                    CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP,
                },
            };
            // SAFETY: a system library mapped for its resources; plain
            // windows of a system class, made and destroyed here; reads this
            // process's GDI object count.
            unsafe {
                let module = LoadLibraryExW(
                    windows::core::w!("user32.dll"),
                    None,
                    LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
                )
                .unwrap();
                let gdi = || GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
                let window = || {
                    CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        windows::core::w!("STATIC"),
                        None,
                        WS_POPUP,
                        0,
                        0,
                        10,
                        10,
                        None,
                        None,
                        None,
                        None,
                    )
                    .unwrap()
                };
                // Whatever the system keeps once per process, made first.
                let first = window();
                set_icons(first, module);
                release_icons(first);
                let _ = DestroyWindow(first);
                let before = gdi();
                for _ in 0..50 {
                    let hwnd = window();
                    set_icons(hwnd, module);
                    set_icons(hwnd, module);
                    release_icons(hwnd);
                    let _ = DestroyWindow(hwnd);
                }
                let after = gdi();
                assert!(after <= before + 2, "{before} GDI objects, then {after}");
                // The count sees the leak: icons left on destroyed windows.
                for _ in 0..10 {
                    let hwnd = window();
                    set_icons(hwnd, module);
                    let _ = DestroyWindow(hwnd);
                }
                let leaked = gdi();
                assert!(leaked >= after + 20, "{after}, then {leaked}");
            }
        }

        /// The system's settings changing reaches the host (`Caption::
        /// settings_changed`, then `system_colors::sync`): how Windows' colour
        /// settings, transparency effects among them, are read again.
        #[test]
        fn a_settings_change_wakes_the_host_to_read_them_again() {
            use windows::Win32::UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP,
            };
            let woken = Rc::new(Cell::new(0));
            let wake = woken.clone();
            let caption = Rc::new(Caption {
                frameless: false,
                buttons: false,
                rtl: false,
                shown: Cell::new(true),
                height: None,
                zoom: Cell::new(1.),
                resizable: Cell::new(true),
                fixed_size: Cell::new(false),
                settings_changed: Cell::new(false),
                hovered: Cell::new(None),
                pressed: Cell::new(None),
                tracking: Cell::new(false),
                wake: RefCell::new(Some(Box::new(move || wake.set(wake.get() + 1)))),
            });
            // SAFETY: a plain window of a system class, made and destroyed
            // here, with the subclass `apply` installs and its reference,
            // which it takes back on WM_NCDESTROY.
            unsafe {
                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    windows::core::w!("STATIC"),
                    None,
                    WS_POPUP,
                    0,
                    0,
                    10,
                    10,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
                let data = Rc::into_raw(caption.clone()) as usize;
                assert!(SetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, data).as_bool());
                SendMessageW(hwnd, WM_SETTINGCHANGE, None, None);
                assert!(caption.settings_changed.get());
                assert_eq!(woken.get(), 1);
                let _ = DestroyWindow(hwnd);
            }
            assert_eq!(Rc::strong_count(&caption), 1, "given back with the window");
        }

        #[test]
        fn a_window_larger_than_the_work_area_shrinks_and_moves_inside_it() {
            let work = (0, 0, 1920, 1032);
            // 1770 by 1140 at (208, 208): too tall, and off the right.
            assert_eq!(
                fit((208, 208, 1770, 1140), work, (0, 0)),
                (150, 0, 1770, 1032)
            );
            // A window that fits stays as it is.
            assert_eq!(
                fit((100, 100, 800, 600), work, (0, 0)),
                (100, 100, 800, 600)
            );
            // Never below the app's minimum, even past the work area.
            assert_eq!(
                fit((0, 0, 3000, 2000), work, (2000, 1200)),
                (0, 0, 2000, 1200)
            );
            // A second monitor's work area, left of the primary.
            assert_eq!(
                fit((-1700, 50, 1600, 900), (-1600, 0, 1600, 860), (0, 0)),
                (-1600, 0, 1600, 860)
            );
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::{ButtonLayout, CaptionHost, resize_cursor};

#[cfg(target_os = "linux")]
mod linux {
    //! Linux title bars, drawn by the app as GTK's header bars are (client
    //! side). `Titlebar::Hidden` and `Titlebar::Inset` are undecorated
    //! windows that zgx resizes from bands along their edges (winit's
    //! `drag_resize_window`: the window manager or compositor moves the
    //! edge). `Titlebar::Inset` adds the window buttons over the content's
    //! top corners, in the order and on the side the desktop asks (GNOME's
    //! `button-layout`, through the settings portal, as GTK 4 reads it),
    //! looking as libadwaita's: 24-point circles centred in the 40-point
    //! strip, a faint fill that deepens under the pointer.
    use std::sync::{Arc, Weak};
    use winit::window::{CursorIcon, ResizeDirection, Window};
    use zgx::{
        compose::prelude::*,
        reactive::Signal,
        style::Styled,
        theme::{Field, tok},
        widgets::Ui,
    };

    /// Resize bands inside an undecorated window's edges, in logical pixels:
    /// it has no frame outside them to take the pointer.
    const RESIZE_BAND: f32 = 5.;
    /// Along an edge, how far from a corner the corner's diagonal resize
    /// reaches, as GTK's: a corner is easier to catch than a 5-point square.
    const CORNER: f32 = 16.;
    /// libadwaita's window buttons: circles of this size, this far apart and
    /// from the window's edge, centred in `Titlebar::Inset`'s strip.
    const BUTTON: f32 = 24.;
    const GAP: f32 = 8.;
    const MARGIN: f32 = 8.;
    const STRIP: f32 = 40.;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum WindowButton {
        Minimize,
        Maximize,
        Close,
    }

    /// Which window buttons go on which side, in order.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct ButtonLayout {
        pub left: [Option<WindowButton>; 3],
        pub right: [Option<WindowButton>; 3],
    }

    impl Default for ButtonLayout {
        /// Minimize, maximize and close on the right: KDE's, Ubuntu's and
        /// most window managers' (GNOME's own asks for close alone).
        fn default() -> Self {
            Self {
                left: [None; 3],
                right: [
                    Some(WindowButton::Minimize),
                    Some(WindowButton::Maximize),
                    Some(WindowButton::Close),
                ],
            }
        }
    }

    impl ButtonLayout {
        /// GNOME's `button-layout` (and GTK's `gtk-decoration-layout`):
        /// the buttons left of the colon, then right of it, by name
        /// (`"appmenu:minimize,maximize,close"`); the names zgx draws no
        /// button for (`appmenu`, `icon`, `spacer`) are left out.
        pub fn parse(layout: &str) -> Self {
            let side = |names: &str| {
                let mut buttons = [None; 3];
                let mut found = names
                    .split(',')
                    .filter_map(|name| match name.trim() {
                        "minimize" => Some(WindowButton::Minimize),
                        "maximize" => Some(WindowButton::Maximize),
                        "close" => Some(WindowButton::Close),
                        _ => None,
                    })
                    .fuse();
                for slot in &mut buttons {
                    *slot = found.next();
                }
                buttons
            };
            let (left, right) = layout.split_once(':').unwrap_or(("", layout));
            Self {
                left: side(left),
                right: side(right),
            }
        }
    }

    /// The edge or corner at (`x`, `y`) of a window `width` by `height`
    /// points, if the pointer there resizes it.
    pub(super) fn edge(x: f32, y: f32, width: f32, height: f32) -> Option<ResizeDirection> {
        // Outside the window: a press a pointer grab reports (a popup's).
        if !(0. ..width).contains(&x) || !(0. ..height).contains(&y) {
            return None;
        }
        let near = |at: f32, size: f32, reach: f32| (at < reach, at >= size - reach);
        let (left, right) = near(x, width, RESIZE_BAND);
        let (top, bottom) = near(y, height, RESIZE_BAND);
        let (corner_x, corner_y) = (near(x, width, CORNER), near(y, height, CORNER));
        use ResizeDirection::*;
        Some(match () {
            _ if (top || left) && corner_x.0 && corner_y.0 => NorthWest,
            _ if (top || right) && corner_x.1 && corner_y.0 => NorthEast,
            _ if (bottom || left) && corner_x.0 && corner_y.1 => SouthWest,
            _ if (bottom || right) && corner_x.1 && corner_y.1 => SouthEast,
            _ if top => North,
            _ if bottom => South,
            _ if left => West,
            _ if right => East,
            _ => return None,
        })
    }

    /// The cursor a resize toward `direction` shows.
    pub(crate) fn resize_cursor(direction: ResizeDirection) -> CursorIcon {
        use ResizeDirection::*;
        match direction {
            North => CursorIcon::NResize,
            South => CursorIcon::SResize,
            East => CursorIcon::EResize,
            West => CursorIcon::WResize,
            NorthEast => CursorIcon::NeResize,
            NorthWest => CursorIcon::NwResize,
            SouthEast => CursorIcon::SeResize,
            SouthWest => CursorIcon::SwResize,
        }
    }

    /// The strip `Inset` keeps and the window buttons in it, in points:
    /// the buttons where `CaptionHost` draws them, on the side the desktop
    /// asks, and the strip between (`WindowInfo::titlebar_area`, `controls`).
    pub(super) fn titlebar_area(
        width: f32,
        titlebar: super::Titlebar,
    ) -> (Option<zgx::scene::Rect>, Option<zgx::scene::Rect>) {
        area_for(
            width,
            titlebar,
            crate::linux_appearance::current()
                .buttons
                .unwrap_or_default(),
        )
    }

    /// `titlebar_area` for `layout`. With buttons on both sides, `controls`
    /// is the right side's (the web's window controls overlay has one) and
    /// the area is the strip between.
    pub(super) fn area_for(
        width: f32,
        titlebar: super::Titlebar,
        layout: ButtonLayout,
    ) -> (Option<zgx::scene::Rect>, Option<zgx::scene::Rect>) {
        use zgx::scene::Rect;
        if titlebar != super::Titlebar::Inset {
            return (Some(Rect::new(0., 0., width, STRIP)), None);
        }
        // A side's buttons with the margins around them.
        let span = |side: [Option<WindowButton>; 3]| match side.iter().flatten().count() {
            0 => 0.,
            n => 2. * MARGIN + n as f32 * BUTTON + (n - 1) as f32 * GAP,
        };
        let left = span(layout.left).min(width);
        let right = span(layout.right).min(width - left);
        let area = Rect::new(left, 0., width - left - right, STRIP);
        let controls = if right > 0. {
            Some(Rect::new(width - right, 0., right, STRIP))
        } else {
            (left > 0.).then(|| Rect::new(0., 0., left, STRIP))
        };
        (Some(area), controls)
    }

    /// An undecorated window's side of its title bar: the window buttons of
    /// `Titlebar::Inset` over its content (the window's state and the
    /// desktop's layout follow, a frame after they change), and the resize
    /// bands along its edges.
    pub(crate) struct CaptionHost {
        window: Weak<Window>,
        close: crate::WindowHandle,
        buttons: bool,
        layout: ButtonLayout,
        maximized: Signal<bool>,
        active: Signal<bool>,
        view: Option<zgx::compose::ViewHandle>,
    }

    impl CaptionHost {
        /// For a window `titlebar` made undecorated; `None` for a standard
        /// one, which the platform decorates.
        pub(crate) fn new(
            window: &Arc<Window>,
            ui: &Ui,
            titlebar: super::Titlebar,
            close: crate::WindowHandle,
        ) -> Option<Self> {
            if titlebar == super::Titlebar::Standard {
                return None;
            }
            Some(Self {
                window: Arc::downgrade(window),
                close,
                buttons: titlebar == super::Titlebar::Inset,
                layout: ButtonLayout::default(),
                maximized: ui.signal(false),
                active: ui.signal(true),
                view: None,
            })
        }

        /// Before a frame and when the window's state changes: bring the
        /// buttons up to date, and back if the content was rendered again
        /// over them.
        pub(crate) fn sync(&mut self, ui: &mut Ui) {
            if !self.buttons {
                return;
            }
            if let Some(window) = self.window.upgrade() {
                self.maximized.set(window.is_maximized());
                self.active.set(window.has_focus());
            }
            let layout = crate::linux_appearance::current()
                .buttons
                .unwrap_or_default();
            if layout != self.layout {
                self.layout = layout;
                if let Some(view) = self.view.take() {
                    view.unmount();
                }
            }
            if !self.view.as_ref().is_some_and(|view| view.is_mounted()) {
                self.view = Some(ui.mount(self.buttons()));
            }
        }

        /// The resize the pointer at (`x`, `y`) would start, if any: none
        /// while the window is maximized, fullscreen or of a fixed size.
        pub(crate) fn edge(&self, x: f32, y: f32, scale: f64) -> Option<ResizeDirection> {
            let window = self.window.upgrade()?;
            if !window.is_resizable() || window.is_maximized() || window.fullscreen().is_some() {
                return None;
            }
            let size = window.inner_size().to_logical::<f32>(scale);
            edge(x, y, size.width, size.height)
        }

        fn buttons(&self) -> View {
            let side = |buttons: [Option<WindowButton>; 3], right: bool| {
                let strip = row()
                    .absolute()
                    .top((STRIP - BUTTON) / 2.)
                    .gap(GAP)
                    .children(buttons.into_iter().flatten().map(|kind| self.button(kind)));
                if right {
                    strip.right(MARGIN)
                } else {
                    strip.left(MARGIN)
                }
            };
            div()
                .absolute()
                .top(0.)
                .left(0.)
                .w_full()
                .h(STRIP)
                .pointer_events(false)
                .child(side(self.layout.left, false))
                .child(side(self.layout.right, true))
        }

        fn button(&self, kind: WindowButton) -> View {
            let window = self.window.clone();
            let close = self.close.clone();
            let maximized = self.maximized.clone();
            // libadwaita's: the text colour at 10% over what is behind,
            // 15% under the pointer, 30% pressed (mixed into the window's
            // background, where these sit).
            let fill = |t: f32| {
                move |s: Styles| {
                    s.token_mix(Field::Background, tok::BACKGROUND, tok::TEXT_PRIMARY, t)
                }
            };
            button()
                .size(BUTTON, BUTTON)
                .p(0.)
                .border(0.)
                .rounded(BUTTON / 2.)
                .items_center()
                .justify_center()
                .shadow_none()
                .focusable(false)
                .pointer_events(true)
                .token_mix(Field::Background, tok::BACKGROUND, tok::TEXT_PRIMARY, 0.1)
                .hover(fill(0.15))
                .active(fill(0.3))
                .accessible_label(match kind {
                    WindowButton::Minimize => "Minimize",
                    WindowButton::Maximize => "Maximize",
                    WindowButton::Close => "Close",
                })
                .on_click(move || {
                    let Some(window) = window.upgrade() else {
                        return;
                    };
                    match kind {
                        WindowButton::Minimize => window.set_minimized(true),
                        WindowButton::Maximize => window.set_maximized(!window.is_maximized()),
                        // As the window manager's close: the app may refuse.
                        WindowButton::Close => close.request_close(),
                    }
                })
                .child(self.glyph(kind, maximized))
        }

        /// The button's symbol, drawn in the text colour (dimmer while the
        /// window is in the back), as Adwaita's symbolic icons.
        fn glyph(&self, kind: WindowButton, maximized: Signal<bool>) -> View {
            let active = self.active.clone();
            let ink = move |s: Styles| {
                s.bg(if active.get() {
                    tok::TEXT_PRIMARY
                } else {
                    tok::TEXT_TERTIARY
                })
            };
            let bar = |w: f32, h: f32| div().size(w, h).rounded(h / 2.);
            match kind {
                WindowButton::Minimize => div()
                    .size(10., 10.)
                    .items_center()
                    .justify_end()
                    .flex_col()
                    .child(bar(8., 2.).reactive_style(move || ink(Styles::new()))),
                WindowButton::Maximize => {
                    let active = self.active.clone();
                    div().reactive_style(move || {
                        let size = if maximized.get() { 7. } else { 9. };
                        Styles::new()
                            .size(size, size)
                            .rounded(1.5)
                            .border(1.5)
                            .border_color(if active.get() {
                                tok::TEXT_PRIMARY
                            } else {
                                tok::TEXT_TERTIARY
                            })
                    })
                }
                WindowButton::Close => {
                    let line = |turn: f32| {
                        let ink = ink.clone();
                        bar(11., 2.)
                            .absolute()
                            .left(0.)
                            .top(4.5)
                            .rotate(turn)
                            .reactive_style(move || ink(Styles::new()))
                    };
                    div()
                        .size(11., 11.)
                        .child(line(std::f32::consts::FRAC_PI_4))
                        .child(line(-std::f32::consts::FRAC_PI_4))
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{ButtonLayout, WindowButton::*, edge};
        use winit::window::ResizeDirection::*;

        #[test]
        fn button_layouts_read_as_gnome_writes_them() {
            let gnome = ButtonLayout::parse("appmenu:close");
            assert_eq!(gnome.left, [None; 3]);
            assert_eq!(gnome.right, [Some(Close), None, None]);
            let left = ButtonLayout::parse("close,minimize,maximize:");
            assert_eq!(left.left, [Some(Close), Some(Minimize), Some(Maximize)]);
            assert_eq!(left.right, [None; 3]);
            assert_eq!(
                ButtonLayout::parse("icon:minimize,spacer,maximize,close"),
                ButtonLayout::default()
            );
            assert_eq!(
                ButtonLayout::parse("minimize,maximize,close").right,
                ButtonLayout::default().right
            );
        }

        #[test]
        fn the_title_bar_area_leaves_the_buttons_where_they_are_drawn() {
            use super::{super::Titlebar, area_for};
            use zgx::scene::Rect;
            // Minimize, maximize, close on the right: 8 + 3 × 24 + 2 × 8 + 8.
            let (area, controls) = area_for(800., Titlebar::Inset, ButtonLayout::default());
            assert_eq!(controls, Some(Rect::new(696., 0., 104., 40.)));
            assert_eq!(area, Some(Rect::new(0., 0., 696., 40.)));
            // GNOME's own: close alone.
            let gnome = ButtonLayout::parse("appmenu:close");
            let (area, controls) = area_for(800., Titlebar::Inset, gnome);
            assert_eq!(controls, Some(Rect::new(760., 0., 40., 40.)));
            assert_eq!(area, Some(Rect::new(0., 0., 760., 40.)));
            // On the left, the strip starts after them.
            let left = ButtonLayout::parse("close,minimize,maximize:");
            let (area, controls) = area_for(800., Titlebar::Inset, left);
            assert_eq!(controls, Some(Rect::new(0., 0., 104., 40.)));
            assert_eq!(area, Some(Rect::new(104., 0., 696., 40.)));
            // Both sides: the right's are the controls, the strip between.
            let both = ButtonLayout::parse("close:minimize,maximize");
            let (area, controls) = area_for(800., Titlebar::Inset, both);
            assert_eq!(controls, Some(Rect::new(728., 0., 72., 40.)));
            assert_eq!(area, Some(Rect::new(40., 0., 688., 40.)));
            // No buttons: the whole strip.
            let (area, controls) = area_for(800., Titlebar::Hidden, ButtonLayout::default());
            assert_eq!((area, controls), (Some(Rect::new(0., 0., 800., 40.)), None));
        }

        #[test]
        fn edges_resize_and_corners_reach_further() {
            let at = |x, y| edge(x, y, 800., 600.);
            assert_eq!(at(400., 300.), None);
            assert_eq!(at(400., 2.), Some(North));
            assert_eq!(at(400., 598.), Some(South));
            assert_eq!(at(1., 300.), Some(West));
            assert_eq!(at(799., 300.), Some(East));
            assert_eq!(at(2., 2.), Some(NorthWest));
            // Along an edge near a corner: the corner's diagonal.
            assert_eq!(at(12., 598.), Some(SouthWest));
            assert_eq!(at(797., 10.), Some(NorthEast));
            assert_eq!(at(790., 590.), None, "inside, near a corner");
            assert_eq!(at(-3., 300.), None, "outside, as a grab reports a press");
        }
    }
}

/// A double click in a window drag region: what a double click on the title
/// bar does. macOS follows the system setting (zoom, minimize or nothing);
/// elsewhere the window toggles maximized.
/// What a double click on a Linux window's title bar does: GNOME's
/// `action-double-click-titlebar` (GTK's `gtk-titlebar-double-click`, on X11
/// the session's XSETTINGS `Gtk/TitlebarDoubleClick`), as GTK's header bars
/// follow it. `lower` and `toggle-shade`, which winit cannot ask of the
/// window manager, do nothing, as GTK's do on Wayland.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TitlebarAction {
    ToggleMaximize,
    Minimize,
    Menu,
    Nothing,
}
#[cfg(target_os = "linux")]
impl TitlebarAction {
    pub(crate) fn parse(action: &str) -> Self {
        match action {
            "minimize" => Self::Minimize,
            "menu" => Self::Menu,
            "none" | "lower" | "toggle-shade" => Self::Nothing,
            // And `toggle-maximize-horizontally`, `-vertically`.
            _ => Self::ToggleMaximize,
        }
    }
    /// The desktop's: on X11 the session's XSETTINGS first, then the
    /// settings portal's; GNOME's default, toggle-maximize, without either.
    pub(crate) fn current() -> Self {
        crate::x11_settings::current()
            .titlebar_double_click
            .or(crate::linux_appearance::current().titlebar_double_click)
            .unwrap_or(Self::ToggleMaximize)
    }
}

/// A window's full-screen notification, queued (`watch_full_screen`): its
/// window number, what AppKit said, about how long the animation it began
/// takes, and the transition it belongs to (a will's number, which its
/// `Settled` carries).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) type FullScreenNote = (
    isize,
    crate::window_info::FullScreenPhase,
    std::time::Duration,
    u64,
);

/// From now on, AppKit's full-screen notifications for every window are
/// queued for `take_full_screen` and `wake` is called (macOS; elsewhere
/// none come, and `window_info.fullscreen` follows resizes). Once per
/// process: a few observers, nothing per frame.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn watch_full_screen(wake: impl Fn() + 'static) {
    #[cfg(target_os = "macos")]
    mac::watch_full_screen(wake);
    #[cfg(not(target_os = "macos"))]
    let _ = wake;
}

/// The full-screen notifications queued since the last call.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn take_full_screen() -> Vec<FullScreenNote> {
    #[cfg(target_os = "macos")]
    {
        mac::take_full_screen()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// Whether AppKit counts `window` covered (its occlusion state not
/// visible), as it is now; `None` where zgx can't ask (not macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn occluded(window: &winit::window::Window) -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        let window = mac::ns_window(window)?;
        Some(
            !window
                .occlusionState()
                .contains(objc2_app_kit::NSWindowOcclusionState::Visible),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        None
    }
}

pub(crate) fn double_click(window: &winit::window::Window) {
    #[cfg(target_os = "macos")]
    mac::double_click(window);
    #[cfg(not(target_os = "macos"))]
    window.set_maximized(!window.is_maximized());
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{Glass, Titlebar};
    use objc2::{
        ClassType, DefinedClass, MainThreadOnly, define_class, msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject},
        sel,
    };
    use objc2_app_kit::{
        NSAutoresizingMaskOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
        NSVisualEffectState, NSVisualEffectView, NSWindow, NSWindowStyleMask,
        NSWindowTitleVisibility,
    };
    use objc2_foundation::NSString;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub(super) fn ns_window(window: &winit::window::Window) -> Option<Retained<NSWindow>> {
        let handle = window.window_handle().ok()?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return None;
        };
        // SAFETY: winit's view, alive while its window is (borrowed here);
        // winit hands out an AppKit handle only on the main thread.
        let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        view.window()
    }

    pub(super) fn apply(
        window: &winit::window::Window,
        titlebar: Titlebar,
        glass: Option<Glass>,
    ) -> f64 {
        let Some(window) = ns_window(window) else {
            return 0.;
        };
        match titlebar {
            Titlebar::Standard => {}
            Titlebar::Inset => {
                content_under_titlebar(&window);
                inset_buttons(&window);
            }
            Titlebar::Hidden => {
                content_under_titlebar(&window);
                // NSWindowCloseButton, MiniaturizeButton, ZoomButton.
                for button in [0_usize, 1, 2] {
                    // SAFETY: `-standardWindowButton:(NSWindowButton)` takes
                    // an NSUInteger and returns an unretained button (or
                    // nil) the window keeps; `setHidden:` takes a BOOL.
                    unsafe {
                        let button: *mut AnyObject =
                            msg_send![&*window, standardWindowButton: button];
                        if let Some(button) = button.as_ref() {
                            let _: () = msg_send![button, setHidden: true];
                        }
                    }
                }
            }
        }
        if let Some(glass) = glass {
            frost(&window, glass);
        }
        radius(&window)
    }

    /// Content under a transparent, untitled title bar with no separator.
    fn content_under_titlebar(window: &NSWindow) {
        window.setStyleMask(window.styleMask() | NSWindowStyleMask::FullSizeContentView);
        window.setTitlebarAppearsTransparent(true);
        window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        // NSTitlebarSeparatorStyleNone
        // SAFETY: the setter takes an NSTitlebarSeparatorStyle (NSInteger).
        unsafe {
            let _: () = msg_send![window, setTitlebarSeparatorStyle: 1_isize];
        }
    }

    /// The inset title bar's toolbar's identifier (`inset_buttons`), by which
    /// full screen finds it (`watch_fullscreen`).
    const INSET_TOOLBAR: &str = "zgx.titlebar.inset";

    /// An empty compact unified toolbar: it centres the traffic lights in a
    /// 40-point strip.
    fn inset_buttons(window: &NSWindow) {
        let Some(class) = AnyClass::get(c"NSToolbar") else {
            return;
        };
        // SAFETY: NSToolbar's `alloc` and `initWithIdentifier:` (the
        // allocation passed on to init, which returns it owned); the BOOL
        // and NSWindowToolbarStyle (NSInteger) setters; the window retains
        // its toolbar.
        unsafe {
            let id = NSString::from_str(INSET_TOOLBAR);
            let toolbar: Retained<AnyObject> =
                msg_send![msg_send![class, alloc], initWithIdentifier: &*id];
            let _: () = msg_send![&*toolbar, setShowsBaselineSeparator: false];
            let _: () = msg_send![window, setToolbar: &*toolbar];
            // NSWindowToolbarStyleUnifiedCompact
            let _: () = msg_send![window, setToolbarStyle: 4_isize];
        }
        toolbar_passes_through();
        watch_fullscreen();
    }

    /// In full screen AppKit moves a window's toolbar into a window of its
    /// own above it (NSToolbarFullScreenWindow), which the title bar's
    /// transparency doesn't reach: the inset title bar's empty toolbar showed
    /// there as an opaque 40-point bar over the content's top. It is hidden as
    /// a window goes into full screen (the green button, `set_fullscreen`)
    /// and shown again once it is out, where it centres the traffic lights
    /// again. A transition that fails after it began (AppKit tells only the
    /// window's delegate, winit's) would leave it hidden on a window that
    /// isn't in full screen: `RESTORE_AFTER` on, one not in full screen gets
    /// it back. One that fails to exit stays in full screen, its toolbar
    /// rightly hidden. Once per process; only zgx's inset toolbar is touched.
    fn watch_fullscreen() {
        use objc2_app_kit::{
            NSWindowDidExitFullScreenNotification, NSWindowWillEnterFullScreenNotification,
        };
        use objc2_foundation::{NSNotification, NSNotificationCenter};
        thread_local! {
            static WATCHING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }
        if WATCHING.replace(true) {
            return;
        }
        let center = NSNotificationCenter::defaultCenter();
        for (name, visible) in [
            // SAFETY: AppKit's own notification names, constant statics.
            (unsafe { NSWindowWillEnterFullScreenNotification }, false),
            (unsafe { NSWindowDidExitFullScreenNotification }, true),
        ] {
            let block = block2::RcBlock::new(move |note: std::ptr::NonNull<NSNotification>| {
                // SAFETY: AppKit passes a valid notification for the
                // block's duration; a window's notification's object is
                // that NSWindow.
                let note = unsafe { note.as_ref() };
                let Some(object) = note.object() else {
                    return;
                };
                let Ok(window) = object.downcast::<NSWindow>() else {
                    return;
                };
                inset_toolbar_visible(&window, visible);
                if !visible {
                    restore_unless_fullscreen(window.windowNumber());
                }
            });
            // SAFETY: with no queue given the block runs where AppKit posts
            // the notification (the main thread); the observers live as
            // long as the app.
            let observer = unsafe {
                center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
            };
            std::mem::forget(observer);
        }
    }

    thread_local! {
        static FULL_SCREEN: std::cell::RefCell<Vec<super::FullScreenNote>> =
            const { std::cell::RefCell::new(Vec::new()) };
        static FULL_SCREEN_WAKE: std::cell::RefCell<Option<std::rc::Rc<dyn Fn()>>> =
            const { std::cell::RefCell::new(None) };
        static FULL_SCREEN_SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
        /// How long the last transition each way took, will to did (going
        /// in, then out): AppKit says nothing of its full-screen animation's
        /// length.
        static FULL_SCREEN_TOOK: std::cell::Cell<[Option<std::time::Duration>; 2]> =
            const { std::cell::Cell::new([None; 2]) };
        /// When each window's transition began, by window number.
        static FULL_SCREEN_BEGAN: std::cell::RefCell<Vec<(isize, std::time::Instant)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    /// Every window's `NSWindowWill…`/`NSWindowDid…FullScreen`, queued with
    /// the window's number (`super::watch_full_screen`). A will also queues
    /// `Settled` once its animation has had time to end, as a transition
    /// that fails says so only to winit's delegate.
    pub(super) fn watch_full_screen(wake: impl Fn() + 'static) {
        use crate::window_info::FullScreenPhase;
        use objc2_app_kit::{
            NSWindowDidEnterFullScreenNotification, NSWindowDidExitFullScreenNotification,
            NSWindowWillEnterFullScreenNotification, NSWindowWillExitFullScreenNotification,
        };
        use objc2_foundation::{NSNotification, NSNotificationCenter};
        if FULL_SCREEN_WAKE.with_borrow(Option::is_some) {
            return;
        }
        FULL_SCREEN_WAKE.set(Some(std::rc::Rc::new(wake)));
        let center = NSNotificationCenter::defaultCenter();
        for (name, phase) in [
            // SAFETY: AppKit's own notification names, constant statics.
            (
                unsafe { NSWindowWillEnterFullScreenNotification },
                FullScreenPhase::WillEnter,
            ),
            (
                unsafe { NSWindowDidEnterFullScreenNotification },
                FullScreenPhase::DidEnter,
            ),
            (
                unsafe { NSWindowWillExitFullScreenNotification },
                FullScreenPhase::WillExit,
            ),
            (
                unsafe { NSWindowDidExitFullScreenNotification },
                FullScreenPhase::DidExit,
            ),
        ] {
            let block = block2::RcBlock::new(move |note: std::ptr::NonNull<NSNotification>| {
                // SAFETY: AppKit passes a valid notification for the
                // block's duration; a window's notification's object is
                // that NSWindow.
                let note = unsafe { note.as_ref() };
                let Some(window) = note
                    .object()
                    .and_then(|object| object.downcast::<NSWindow>().ok())
                else {
                    return;
                };
                let number = window.windowNumber();
                let will = matches!(
                    phase,
                    FullScreenPhase::WillEnter | FullScreenPhase::WillExit
                );
                let way = usize::from(matches!(
                    phase,
                    FullScreenPhase::WillExit | FullScreenPhase::DidExit
                ));
                let (duration, seq) = if will {
                    // As long as the last one this way took; the first, as
                    // long as AppKit's takes (`FULL_SCREEN_FIRST`).
                    let duration = FULL_SCREEN_TOOK.get()[way].unwrap_or(FULL_SCREEN_FIRST);
                    FULL_SCREEN_BEGAN.with_borrow_mut(|began| {
                        began.retain(|(window, _)| *window != number);
                        began.push((number, std::time::Instant::now()));
                    });
                    let seq = FULL_SCREEN_SEQ.get() + 1;
                    FULL_SCREEN_SEQ.set(seq);
                    (duration, seq)
                } else {
                    // Done: how long it took, for the next this way.
                    let began = FULL_SCREEN_BEGAN.with_borrow_mut(|began| {
                        let at = began.iter().position(|(window, _)| *window == number)?;
                        Some(began.swap_remove(at).1)
                    });
                    if let Some(began) = began {
                        let mut took = FULL_SCREEN_TOOK.get();
                        took[way] = Some(began.elapsed());
                        FULL_SCREEN_TOOK.set(took);
                    }
                    (std::time::Duration::ZERO, FULL_SCREEN_SEQ.get())
                };
                note_full_screen((number, phase, duration, seq));
                if will {
                    // Past its animation, and past an inset toolbar's own
                    // restore (`RESTORE_AFTER`): the buttons read then are
                    // where they stay.
                    let after = (duration + SETTLE_AFTER).max(RESTORE_AFTER + SETTLE_MARGIN);
                    settle_after(number, after, seq);
                }
            });
            // SAFETY: with no queue given the block runs where AppKit posts
            // the notification (the main thread); the observers live as
            // long as the app.
            let observer = unsafe {
                center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
            };
            std::mem::forget(observer);
        }
    }

    /// How long a window's first transition each way is said to take: about
    /// what AppKit's took on macOS 26 (0.52 to 0.58 s, in and out).
    /// `animationResizeTime`, a resize's, says 0.35 s in and 0.07 s out.
    const FULL_SCREEN_FIRST: std::time::Duration = std::time::Duration::from_millis(550);
    /// How long past a transition's estimated end its `Settled` comes, and
    /// past an inset toolbar's restore at the soonest.
    const SETTLE_AFTER: std::time::Duration = std::time::Duration::from_millis(1500);
    const SETTLE_MARGIN: std::time::Duration = std::time::Duration::from_millis(250);

    fn note_full_screen(note: super::FullScreenNote) {
        FULL_SCREEN.with_borrow_mut(|queue| queue.push(note));
        if let Some(wake) = FULL_SCREEN_WAKE.with_borrow(Clone::clone) {
            wake();
        }
    }

    /// `after` on, the transition `seq` of the window numbered `number` is
    /// over, if it is not yet.
    fn settle_after(number: isize, after: std::time::Duration, seq: u64) {
        let Ok(when) = dispatch2::DispatchTime::try_from(after) else {
            return;
        };
        let _ = dispatch2::DispatchQueue::main().after(when, move || {
            note_full_screen((
                number,
                crate::window_info::FullScreenPhase::Settled,
                std::time::Duration::ZERO,
                seq,
            ));
        });
    }

    pub(super) fn take_full_screen() -> Vec<super::FullScreenNote> {
        FULL_SCREEN.with_borrow_mut(std::mem::take)
    }

    /// How long after a window began going into full screen its transition
    /// is over, if it is: AppKit's animation takes about half a second.
    const RESTORE_AFTER: std::time::Duration = std::time::Duration::from_secs(2);

    /// `RESTORE_AFTER` on, the window numbered `number` gets its inset
    /// toolbar back unless it is in full screen (its transition failed).
    fn restore_unless_fullscreen(number: isize) {
        let Ok(when) = dispatch2::DispatchTime::try_from(RESTORE_AFTER) else {
            return;
        };
        let _ = dispatch2::DispatchQueue::main().after(when, move || {
            let Some(mtm) = objc2::MainThreadMarker::new() else {
                return;
            };
            let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
            let Some(window) = app.windowWithWindowNumber(number) else {
                return;
            };
            if !window.styleMask().contains(NSWindowStyleMask::FullScreen) {
                inset_toolbar_visible(&window, true);
            }
        });
    }

    /// Show or hide `window`'s toolbar if it is the inset title bar's.
    fn inset_toolbar_visible(window: &NSWindow, visible: bool) {
        // SAFETY: `-toolbar` returns the window's NSToolbar or nil, retained
        // here; `-identifier` an NSString; `-setVisible:` takes a BOOL.
        unsafe {
            let toolbar: Option<Retained<AnyObject>> = msg_send![window, toolbar];
            let Some(toolbar) = toolbar else {
                return;
            };
            let id: Option<Retained<NSString>> = msg_send![&*toolbar, identifier];
            if id.is_some_and(|id| id.to_string() == INSET_TOOLBAR) {
                let _: () = msg_send![&*toolbar, setVisible: visible];
            }
        }
    }

    /// The empty toolbar is only there to centre the traffic lights: presses
    /// in it go to the content under it. Otherwise it keeps a press's drag
    /// and release events (zgx would start a drag and never see it end).
    /// The traffic lights are in the titlebar view, not the toolbar's, and
    /// stay clickable.
    fn toolbar_passes_through() {
        use objc2::runtime::Sel;
        use objc2_foundation::NSPoint;
        extern "C-unwind" fn nothing(
            _this: *mut AnyObject,
            _cmd: Sel,
            _point: NSPoint,
        ) -> *mut AnyObject {
            std::ptr::null_mut()
        }
        let Some(class) = AnyClass::get(c"NSToolbarView") else {
            return;
        };
        // SAFETY: `nothing` has `hitTest:`'s C signature (self, _cmd, an
        // NSPoint by value, returning an object), which the type encoding
        // states; the runtime calls an `Imp` through that signature, so the
        // transmute only erases the type of a function pointer. It returns
        // nil, a valid `hitTest:` answer. `class_replaceMethod` is
        // thread-safe and keeps nothing of ours but the function.
        unsafe {
            let imp: objc2::runtime::Imp = std::mem::transmute(
                nothing as extern "C-unwind" fn(*mut AnyObject, Sel, NSPoint) -> *mut AnyObject,
            );
            objc2::ffi::class_replaceMethod(
                class as *const AnyClass as *mut AnyClass,
                sel!(hitTest:),
                imp,
                c"@@:{CGPoint=dd}".as_ptr(),
            );
        }
    }

    /// The window's corner radius: it varies by macOS version and toolbar
    /// style.
    pub(super) fn radius(window: &NSWindow) -> f64 {
        if window.class().responds_to(sel!(_cornerRadius)) {
            // SAFETY: an unpublished getter, sent only where it exists, that
            // takes nothing; that it returns a CGFloat is AppKit's private
            // declaration (and other apps' use of it), not something
            // checked here.
            unsafe { msg_send![window, _cornerRadius] }
        } else {
            10.
        }
    }

    pub(crate) struct GlassState {
        plain: bool,
    }
    // SAFETY: an NSVisualEffectView subclass, main-thread only as views are,
    // with nothing to clean up beyond its ivars; `updateLayer` has AppKit's
    // signature (`-(void)updateLayer`).
    define_class!(
        /// A behind-window effect view that, plain, strips the layers AppKit
        /// builds for it each time it builds them (gpui's `BlurredView`).
        #[unsafe(super(NSVisualEffectView, NSView))]
        #[thread_kind = MainThreadOnly]
        #[name = "ZgxWindowGlass"]
        #[ivars = GlassState]
        pub(crate) struct GlassView;
        impl GlassView {
            #[unsafe(method(updateLayer))]
            fn update_layer(&self) {
                // SAFETY: the superclass's own `updateLayer`.
                let _: () = unsafe { msg_send![super(self), updateLayer] };
                if self.ivars().plain
                    && let Some(layer) = self.layer()
                {
                    // SAFETY: the view's own CALayer, on the main thread.
                    unsafe { strip_layer(&layer) };
                }
            }
        }
    );

    /// The window's glass replaced (`super::set_glass`): the effect view
    /// `frost` put under the content goes, and `glass`'s takes its place.
    pub(super) fn set_glass(window: &winit::window::Window, glass: Option<Glass>) {
        use objc2::runtime::NSObjectProtocol as _;
        let Some(window) = ns_window(window) else {
            return;
        };
        // SAFETY: the window's frame view, kept while it holds its content.
        let Some(frame) = window
            .contentView()
            .and_then(|content| unsafe { content.superview() })
        else {
            return;
        };
        for view in frame.subviews() {
            if view.isKindOfClass(GlassView::class()) {
                view.removeFromSuperview();
            }
        }
        match glass {
            Some(glass) => frost(&window, glass),
            None => window.invalidateShadow(),
        }
    }

    /// Put a behind-window effect view under the (transparent) zgx view.
    /// Unlike the private CGS window blur, it can be clipped to the window's
    /// rounded shape, so neither blur nor shadow is left square.
    fn frost(window: &NSWindow, glass: Glass) {
        let Some(content) = window.contentView() else {
            return;
        };
        // SAFETY: the window's frame view, which the window keeps while it
        // holds its content view.
        let Some(frame) = (unsafe { content.superview() }) else {
            return;
        };
        let mtm = window.mtm();
        // SAFETY: NSView's designated initializer, on a fresh allocation
        // with its ivars set.
        let effect: Retained<GlassView> = unsafe {
            let object = mtm
                .alloc::<GlassView>()
                .set_ivars(GlassState { plain: glass.plain });
            msg_send![super(object), initWithFrame: frame.bounds()]
        };
        effect.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        // Behind-window blending, always active.
        effect.setMaterial(NSVisualEffectMaterial(glass.material.appkit_value()));
        effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        effect.setState(NSVisualEffectState::Active);
        effect.setWantsLayer(true);
        if let Some(layer) = effect.layer() {
            layer.setCornerRadius(radius(window));
            // SAFETY: `setCornerCurve:` takes a CALayerCornerCurve, an
            // NSString ("continuous" is `kCACornerCurveContinuous`'s value).
            unsafe {
                let continuous = NSString::from_str("continuous");
                let _: () = msg_send![&*layer, setCornerCurve: &*continuous];
            }
            layer.setMasksToBounds(true);
            if glass.plain {
                // SAFETY: the view's own CALayer, on the main thread.
                unsafe { strip_layer(&layer) };
            }
        }
        frame.addSubview_positioned_relativeTo(
            &effect,
            objc2_app_kit::NSWindowOrderingMode::Below,
            Some(&content),
        );
        window.invalidateShadow();
    }

    /// The title bar's free strip and the traffic lights in it, in points
    /// from the content's top left. From the buttons' own frames and the
    /// title bar's own height, so a toolbar's taller bar, a right-to-left
    /// layout (buttons at the right, the strip left of them) and hidden
    /// buttons come out as AppKit lays them out. None in fullscreen.
    /// AppKit's own places for a window's traffic lights, in its frame's
    /// points: the first's left, the step from one to the next, the
    /// centres' distance below the top, and the title bar container's
    /// height. Kept from before zgx first moves them.
    #[derive(Clone, Copy)]
    struct Buttons {
        left: f64,
        step: f64,
        centre: f64,
        container: f64,
    }
    thread_local! {
        static BUTTONS: std::cell::RefCell<std::collections::HashMap<isize, Buttons>> =
            Default::default();
    }

    pub(super) fn zoom_buttons(window: &winit::window::Window, zoom: f64) {
        use objc2_foundation::{NSPoint, NSRect, NSSize};
        let Some(window) = ns_window(window) else {
            return;
        };
        if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
            return;
        }
        // NSWindowCloseButton, MiniaturizeButton, ZoomButton.
        let buttons: Vec<Retained<NSView>> = [0_usize, 1, 2]
            .into_iter()
            .filter_map(|button| {
                // SAFETY: `-standardWindowButton:` takes an NSUInteger and
                // returns an NSButton (a view) or nil, retained here.
                unsafe { msg_send![&*window, standardWindowButton: button] }
            })
            .collect();
        let [close, next, ..] = buttons.as_slice() else {
            return;
        };
        let height = window.frame().size.height;
        // A view's frame in the window's (bottom-up) points.
        let in_window = |view: &NSView| view.convertRect_toView(view.bounds(), None);
        // SAFETY: a superview clears its subviews' back pointer as it lets
        // them go, so one still set is alive (main thread).
        let Some(container) =
            (unsafe { close.superview() }).and_then(|bar| unsafe { bar.superview() })
        else {
            return;
        };
        // AppKit's own layout first (a resize leaves it pending): done now, it
        // has nothing left to put back over what follows.
        container.layoutSubtreeIfNeeded();
        let number = window.windowNumber();
        let base = BUTTONS.with_borrow_mut(|known| {
            *known.entry(number).or_insert_with(|| {
                let (first, second) = (in_window(close), in_window(next));
                Buttons {
                    left: first.origin.x,
                    step: second.origin.x - first.origin.x,
                    centre: height - (first.origin.y + first.size.height / 2.),
                    container: container.frame().size.height,
                }
            })
        });
        // Room for them: the container at the window's top (placed from the
        // window's height, not its own frame: a resize moves it later) and at
        // least as tall as they reach.
        let size = close.frame().size;
        let reach = (base.centre * zoom + size.height / 2. + 4.).max(base.container);
        let wanted = if zoom == 1. { base.container } else { reach };
        // SAFETY: as above, a set back pointer is a live superview.
        if let Some(frame_view) = unsafe { container.superview() } {
            let bounds = frame_view.bounds();
            let y = match frame_view.isFlipped() {
                true => 0.,
                false => bounds.size.height - wanted,
            };
            let placed = NSRect::new(NSPoint::new(0., y), NSSize::new(bounds.size.width, wanted));
            if container.frame() != placed {
                container.setFrame(placed);
            }
        }
        for (index, button) in buttons.iter().enumerate() {
            let x = base.left * zoom + base.step * index as f64;
            let y = height - base.centre * zoom - size.height / 2.;
            // SAFETY: as above, a set back pointer is a live superview.
            let Some(bar) = (unsafe { button.superview() }) else {
                continue;
            };
            let origin = bar.convertPoint_fromView(NSPoint::new(x, y), None);
            if button.frame().origin != origin {
                button.setFrameOrigin(origin);
            }
        }
        // Back in AppKit's places: read them again next time (another window
        // may take this one's number).
        if zoom == 1. {
            BUTTONS.with_borrow_mut(|known| known.remove(&number));
        }
    }

    pub(super) fn titlebar_area(
        window: &winit::window::Window,
    ) -> (Option<zgx::scene::Rect>, Option<zgx::scene::Rect>) {
        use zgx::scene::Rect;
        let Some(window) = ns_window(window) else {
            return (None, None);
        };
        let Some(content) = window.contentView() else {
            return (None, None);
        };
        if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
            return (None, None);
        }
        let size = content.frame().size;
        // The content covers the title bar (a full-size content view): the
        // bar is what the content's layout rect leaves out.
        let bar = (size.height - window.contentLayoutRect().size.height) as f32;
        if bar <= 0. {
            return (None, None);
        }
        let width = size.width as f32;
        let mut controls: Option<Rect> = None;
        // NSWindowCloseButton, MiniaturizeButton, ZoomButton.
        for button in [0_usize, 1, 2] {
            // SAFETY: `-standardWindowButton:` takes an NSUInteger and
            // returns an NSButton (a view) or nil, retained here.
            let button: Option<Retained<NSView>> =
                unsafe { msg_send![&*window, standardWindowButton: button] };
            let Some(button) = button.filter(|b| !b.isHidden()) else {
                continue;
            };
            let frame = content.convertRect_fromView(button.bounds(), Some(&button));
            let y = if content.isFlipped() {
                frame.origin.y
            } else {
                size.height - frame.origin.y - frame.size.height
            };
            let rect = Rect::new(
                frame.origin.x as f32,
                y as f32,
                frame.size.width as f32,
                frame.size.height as f32,
            );
            controls = Some(match controls {
                Some(c) => {
                    let (x0, y0) = (c.x.min(rect.x), c.y.min(rect.y));
                    let x1 = (c.x + c.width).max(rect.x + rect.width);
                    let y1 = (c.y + c.height).max(rect.y + rect.height);
                    Rect::new(x0, y0, x1 - x0, y1 - y0)
                }
                None => rect,
            });
        }
        let area = match controls {
            // Right to left: the buttons at the right, the strip before them.
            Some(c) if c.x > width / 2. => Rect::new(0., 0., c.x.max(0.), bar),
            Some(c) => {
                let end = (c.x + c.width).min(width);
                Rect::new(end, 0., width - end, bar)
            }
            None => Rect::new(0., 0., width, bar),
        };
        (Some(area), controls)
    }

    /// No desktop tint (`CAChameleonLayer` hidden), a 60-point gaussian and
    /// no saturation boost, through the effect's layer tree.
    ///
    /// # Safety
    ///
    /// `layer` is a `CALayer` that no other thread uses meanwhile (the
    /// messages below are CALayer's).
    unsafe fn strip_layer(layer: &AnyObject) {
        // SAFETY: CALayer's messages with their declared types:
        // `setBackgroundColor:` a CGColorRef (null: none), `className` an
        // autoreleased NSString, `filters` and `sublayers` unretained
        // NSArrays (or nil) the layer holds while this runs, read by
        // `count` (NSUInteger) and `objectAtIndex:`, each object unretained
        // but held by its array. `arrayWithObjects:count:` reads `count`
        // pointers from `kept`, all live (held by `filters`), and returns
        // an autoreleased array it retains them in. Sublayers are CALayers.
        unsafe {
            // A CGColorRef, not an object: objc2 checks the type in debug.
            let none: *const objc2_core_graphics::CGColor = std::ptr::null();
            let _: () = msg_send![layer, setBackgroundColor: none];
            let class_name: Retained<NSString> = msg_send![layer, className];
            if class_name.to_string() == "CAChameleonLayer" {
                let _: () = msg_send![layer, setHidden: true];
                return;
            }
            let filters: *mut AnyObject = msg_send![layer, filters];
            if let Some(filters) = filters.as_ref() {
                let count: usize = msg_send![filters, count];
                let mut kept: Vec<*mut AnyObject> = Vec::with_capacity(count);
                for i in 0..count {
                    let filter: *mut AnyObject = msg_send![filters, objectAtIndex: i];
                    let Some(filter_ref) = filter.as_ref() else {
                        continue;
                    };
                    let description: Retained<NSString> = msg_send![filter_ref, description];
                    let description = description.to_string();
                    if description.contains("Saturat") {
                        continue;
                    }
                    if description.contains("Blur") {
                        let radius = objc2_foundation::NSNumber::new_f64(60.);
                        let key = NSString::from_str("inputRadius");
                        let _: () = msg_send![filter_ref, setValue: &*radius, forKey: &*key];
                    }
                    kept.push(filter);
                }
                if let Some(array_class) = AnyClass::get(c"NSArray") {
                    let array: Retained<AnyObject> = msg_send![
                        array_class,
                        arrayWithObjects: kept.as_ptr(),
                        count: kept.len()
                    ];
                    let _: () = msg_send![layer, setFilters: &*array];
                }
            }
            let sublayers: *mut AnyObject = msg_send![layer, sublayers];
            if let Some(sublayers) = sublayers.as_ref() {
                let count: usize = msg_send![sublayers, count];
                for i in 0..count {
                    let sublayer: *mut AnyObject = msg_send![sublayers, objectAtIndex: i];
                    if let Some(sublayer) = sublayer.as_ref() {
                        strip_layer(sublayer);
                    }
                }
            }
        }
    }

    /// The system's title bar double click (Desktop & Dock settings).
    pub(super) fn double_click(window: &winit::window::Window) {
        let Some(window) = ns_window(window) else {
            return;
        };
        // SAFETY: NSUserDefaults' `standardUserDefaults` (unretained, the
        // process keeps it), `stringForKey:` (an unretained NSString or nil,
        // read at once) and `boolForKey:` (BOOL).
        let action = unsafe {
            let defaults: *mut AnyObject =
                msg_send![objc2::class!(NSUserDefaults), standardUserDefaults];
            let Some(defaults) = defaults.as_ref() else {
                return;
            };
            let key = NSString::from_str("AppleActionOnDoubleClick");
            let action: *mut NSString = msg_send![defaults, stringForKey: &*key];
            match action.as_ref() {
                Some(action) => action.to_string(),
                None => {
                    // Before the setting had choices: minimize or zoom.
                    let key = NSString::from_str("AppleMiniaturizeOnDoubleClick");
                    let minimize: bool = msg_send![defaults, boolForKey: &*key];
                    if minimize { "Minimize" } else { "Maximize" }.to_owned()
                }
            }
        };
        match action.as_str() {
            "Minimize" => window.miniaturize(None),
            "None" => {}
            // "Maximize" (zoom), "Fill" and anything newer: zoom.
            _ => window.zoom(None),
        }
    }

    #[cfg(test)]
    mod tests {
        use objc2_quartz_core::CALayer;

        /// objc2 checks each message's argument types in a debug build:
        /// stripping a layer tree (as `Glass::plain` does) sends only
        /// well-typed ones (`setBackgroundColor:` takes a CGColorRef).
        #[test]
        fn a_plain_glass_strips_layers_with_well_typed_messages() {
            let (layer, sublayer) = (CALayer::new(), CALayer::new());
            layer.addSublayer(&sublayer);
            // SAFETY: a CALayer only this thread has.
            unsafe { super::strip_layer(&layer) };
            assert!(layer.backgroundColor().is_none());
        }
    }
}
