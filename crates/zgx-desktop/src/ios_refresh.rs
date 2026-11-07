//! An iPhone's display stays at 60 Hz unless something asks for more: a display
//! link asking for 120 Hz, running only while frames are drawn, so an idle app
//! still has nothing awake.
use objc2::{
    MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol},
    sel,
};
use std::{
    cell::{Cell, OnceCell, RefCell},
    time::{Duration, Instant},
};

unsafe extern "C" {
    static NSRunLoopCommonModes: *const AnyObject;
}

/// CoreAnimation's `CAFrameRateRange`.
#[repr(C)]
pub(crate) struct CAFrameRateRange {
    pub minimum: f32,
    pub maximum: f32,
    pub preferred: f32,
}
// SAFETY: three floats, as the struct is declared.
unsafe impl objc2::Encode for CAFrameRateRange {
    const ENCODING: objc2::Encoding = objc2::Encoding::Struct(
        "CAFrameRateRange",
        &[f32::ENCODING, f32::ENCODING, f32::ENCODING],
    );
}

/// Asked for through a gesture's or animation's frames.
pub(crate) const FAST: CAFrameRateRange = CAFrameRateRange {
    minimum: 80.,
    maximum: 120.,
    preferred: 120.,
};

/// How long after the last frame the display may slow again: longer than any
/// gap between one animation's frames.
const LINGER: Duration = Duration::from_millis(150);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct Holder;
    unsafe impl NSObjectProtocol for Holder {}
    impl Holder {
        #[unsafe(method(tick:))]
        fn tick(&self, link: &AnyObject) {
            let quiet = LAST.with(|last| last.get().elapsed() > LINGER);
            if quiet {
                RUNNING.with(|running| running.set(false));
                // SAFETY: CADisplayLink's setter, on the main thread.
                let _: () = unsafe { msg_send![link, setPaused: true] };
            }
        }
    }
);

thread_local! {
    static LINK: OnceCell<Option<Retained<AnyObject>>> = const { OnceCell::new() };
    static LAST: Cell<Instant> = Cell::new(Instant::now());
    static RUNNING: Cell<bool> = const { Cell::new(false) };
    static DRAWS: Cell<Option<(Instant, Duration)>> = const { Cell::new(None) };
    static MARKS: Cell<[Option<Instant>; 3]> = const { Cell::new([None; 3]) };
    /// The gaps between drawn frames, while `record_gaps` asked for them.
    static GAPS: RefCell<Option<Vec<f32>>> = const { RefCell::new(None) };
}

/// Keep the gaps between drawn frames, for `take_gaps`.
pub(crate) fn record_gaps() {
    GAPS.with(|gaps| *gaps.borrow_mut() = Some(Vec::new()));
}

/// The gaps between frames drawn since it last was asked, in milliseconds.
pub(crate) fn take_gaps() -> Vec<f32> {
    GAPS.with(|gaps| gaps.borrow_mut().as_mut().map(std::mem::take))
        .unwrap_or_default()
}

fn make_link(mtm: MainThreadMarker) -> Retained<AnyObject> {
    let holder = Holder::alloc(mtm).set_ivars(());
    // SAFETY: NSObject's `init`; CADisplayLink's constructor and setter; the run
    // loop keeps the link, and the link its target.
    unsafe {
        let holder: Retained<Holder> = msg_send![super(holder), init];
        let link: Retained<AnyObject> = msg_send![class!(CADisplayLink),
            displayLinkWithTarget: &*holder,
            selector: sel!(tick:)];
        let _: () = msg_send![&*link, setPreferredFrameRateRange: FAST];
        let run_loop: Retained<AnyObject> = msg_send![class!(NSRunLoop), mainRunLoop];
        let _: () = msg_send![&*link, addToRunLoop: &*run_loop, forMode: NSRunLoopCommonModes];
        link
    }
}

/// Gaps this long while frames follow each other go in the gesture log, for
/// what a phone in someone's hand does: `(this draw, the last)` says whether the
/// time went in drawing, or before it.
const LONG_GAP: Duration = Duration::from_millis(20);
/// A pass this long is logged by its parts, whatever the gap.
const LONG_DRAW: Duration = Duration::from_millis(10);

/// A point in the pass of drawing under way, for `note_draw`.
pub(crate) fn draw_mark(part: usize) {
    MARKS.with(|marks| {
        let mut now = marks.get();
        now[part] = Some(Instant::now());
        marks.set(now);
    });
}

/// A pass of drawing that began at `began` is over.
pub(crate) fn note_draw(began: Instant) {
    let took = began.elapsed();
    let [prepared, rendered, presented] = MARKS.with(|marks| marks.replace([None; 3]));
    if took >= LONG_DRAW {
        let ms = |from: Option<Instant>, to: Option<Instant>| {
            from.zip(to).map_or(-1., |(from, to)| {
                to.duration_since(from).as_secs_f64() * 1000.
            })
        };
        let end = Some(began + took);
        crate::ios_gestures::log(&format!(
            "long draw {:.1} ms: prepare {:.1}, render {:.1}, present {:.1}, after {:.1}",
            took.as_secs_f64() * 1000.,
            ms(Some(began), prepared),
            ms(prepared, rendered),
            ms(rendered, presented),
            ms(presented, end),
        ));
    }
    let (last_began, last_took) = DRAWS
        .with(|draws| draws.replace(Some((began, took))))
        .unzip();
    if let (Some(last_began), Some(last_took)) = (last_began, last_took) {
        let gap = began.duration_since(last_began);
        if gap >= LONG_GAP && gap < LINGER {
            crate::ios_gestures::log(&format!(
                "long gap {:.1} ms: this draw {:.1}, last {:.1}",
                gap.as_secs_f64() * 1000.,
                took.as_secs_f64() * 1000.,
                last_took.as_secs_f64() * 1000.,
            ));
        }
    }
}

/// A frame was drawn: keep the display fast for the next.
pub(crate) fn frame_drawn() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let previous = LAST.with(|last| last.replace(Instant::now()));
    GAPS.with(|gaps| {
        if let Some(gaps) = gaps.borrow_mut().as_mut() {
            gaps.push(previous.elapsed().as_secs_f32() * 1000.);
        }
    });
    if RUNNING.with(Cell::get) {
        return;
    }
    RUNNING.with(|running| running.set(true));
    LINK.with(|link| {
        let link = link.get_or_init(|| Some(make_link(mtm)));
        if let Some(link) = link {
            // SAFETY: CADisplayLink's setter, on the main thread.
            let _: () = unsafe { msg_send![&**link, setPaused: false] };
        }
    });
}
