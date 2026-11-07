use super::*;
use crate::{animation::Animation, image::ImageData};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
#[derive(Default)]
struct Playback {
    elapsed: Duration,
    started: Option<Instant>,
}
impl Playback {
    fn elapsed(&self) -> Duration {
        self.elapsed + self.started.map_or(Duration::ZERO, crate::clock::since)
    }
    fn pause(&mut self) {
        self.elapsed = self.elapsed();
        self.started = None;
    }
}
pub(crate) struct Animated {
    pub animation: Arc<Animation>,
    pub playing: Option<Signal<bool>>,
}
pub(crate) fn mount(
    ui: &mut Ui,
    root: NodeId,
    source: Signal<Option<Arc<ImageData>>>,
    animation: Animated,
    environment: &Environment,
) {
    if animation.animation.len() < 2 {
        return;
    }
    let visible = ui.observe_visibility(root);
    let presented = ui.observe_presentation();
    let runner = environment
        .services
        .get::<TaskRunner>()
        .expect("animated_image requires a TaskRunner provider");
    let themes = crate::theme::Themes::of(&ui.runtime);
    let playback = Rc::new(RefCell::new(Playback::default()));
    let task: Rc<RefCell<Option<TaskToken>>> = Default::default();
    ui.retain(root, task.clone());
    ui.bind(root, move || {
        // The system's reduce motion (`SystemPreferences`, Luau's
        // `ui.prefers.reduced_motion`): its first frame, still, as a page's
        // `prefers-reduced-motion` asks.
        let still = themes.system_preferences().reduced_motion;
        let active = !still
            && presented.get()
            && visible.get()
            && animation
                .playing
                .as_ref()
                .is_none_or(|playing| playing.get());
        let previous = task.borrow_mut().take();
        drop(previous);
        playback.borrow_mut().pause();
        if still {
            *playback.borrow_mut() = Playback::default();
            source.set(Some(animation.animation.frame(0)));
        }
        if !active {
            return;
        }
        playback.borrow_mut().started = Some(crate::clock::now());
        let data = animation.animation.clone();
        let playback = playback.clone();
        let source = source.clone();
        let future = async move {
            loop {
                let (index, remaining) = data.at(playback.borrow().elapsed());
                source.set(Some(data.frame(index)));
                let Some(remaining) = remaining else {
                    break;
                };
                crate::timer::sleep(remaining).await;
            }
        };
        *task.borrow_mut() = Some((runner.0)(Box::pin(future)));
    });
}
/// An image view's playback of the animations its source carries
/// (`ImageData::animated`, a GIF's still), set up the first time one comes
/// (`follow`): a view of still pictures has none.
pub(crate) struct Followed {
    /// What plays, once set up; until then, `wanted`.
    current: RefCell<Option<Signal<Option<Arc<Animation>>>>>,
    wanted: RefCell<Option<Arc<Animation>>>,
    /// The task setting it up, until it has.
    starting: RefCell<Option<TaskToken>>,
}
/// Play `next` (or stop, for `None`) in image view `root`, whose shown
/// picture is `data`: the playback is set up the first time (on a task of
/// `runner`'s, out of the binding calling this, where nothing can mount).
/// Whether `next` is what plays already (its still shouldn't be shown over
/// the frame it is at). Nothing without a runner: its still shows. Out of
/// line: the binding calling it runs for every picture an image view
/// shows, nearly all of them stills.
#[cold]
#[inline(never)]
pub(crate) fn follow(
    plays: &mut Option<Rc<Followed>>,
    runner: Option<&TaskRunner>,
    ui: &crate::widgets::WeakUi,
    root: NodeId,
    data: &Signal<Option<Arc<ImageData>>>,
    next: Option<Arc<Animation>>,
) -> bool {
    let Some(runner) = runner else {
        return false;
    };
    let same = |a: &Option<Arc<Animation>>| match (a, &next) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    };
    if let Some(followed) = plays {
        if let Some(current) = &*followed.current.borrow() {
            if current.with_untracked(same) {
                return true;
            }
            current.mutate(|playing| {
                *playing = next;
                true
            });
            return false;
        }
        let continuing = same(&followed.wanted.borrow());
        *followed.wanted.borrow_mut() = next;
        return continuing;
    }
    let followed = Rc::new(Followed {
        current: RefCell::new(None),
        wanted: RefCell::new(next),
        starting: RefCell::new(None),
    });
    let (start, ui, data, task_runner) = (
        followed.clone(),
        ui.clone(),
        data.clone(),
        TaskRunner(runner.0.clone()),
    );
    let token = (runner.0)(Box::pin(async move {
        let Some(mut ui) = ui.upgrade() else { return };
        if !ui.scene.borrow().contains(root) {
            return;
        }
        let current = ui.signal(start.wanted.borrow_mut().take());
        mount_followed(&mut ui, root, data, current.clone(), task_runner);
        *start.current.borrow_mut() = Some(current);
    }));
    *followed.starting.borrow_mut() = Some(token);
    *plays = Some(followed);
    false
}
/// `mount`, playing whichever animation `current` holds: from its first
/// frame each time it changes, nothing while it holds none.
fn mount_followed(
    ui: &mut Ui,
    root: NodeId,
    source: Signal<Option<Arc<ImageData>>>,
    current: Signal<Option<Arc<Animation>>>,
    runner: TaskRunner,
) {
    let visible = ui.observe_visibility(root);
    let presented = ui.observe_presentation();
    let themes = crate::theme::Themes::of(&ui.runtime);
    let playback = Rc::new(RefCell::new(Playback::default()));
    let task: Rc<RefCell<Option<TaskToken>>> = Default::default();
    ui.retain(root, task.clone());
    let mut played: Option<Arc<Animation>> = None;
    ui.bind(root, move || {
        let animation = current.get();
        // Reduce motion: its first frame, still (as `mount`'s).
        let still = themes.system_preferences().reduced_motion;
        let active = !still && presented.get() && visible.get();
        drop(task.borrow_mut().take());
        let changed = match (&played, &animation) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        if changed || still {
            *playback.borrow_mut() = Playback::default();
            played = animation.clone();
        } else {
            playback.borrow_mut().pause();
        }
        if still && let Some(animation) = &animation {
            source.set(Some(animation.frame(0)));
        }
        let Some(data) = animation.filter(|_| active) else {
            return;
        };
        playback.borrow_mut().started = Some(crate::clock::now());
        let playback = playback.clone();
        let source = source.clone();
        let future = async move {
            loop {
                let (index, remaining) = data.at(playback.borrow().elapsed());
                source.set(Some(data.frame(index)));
                let Some(remaining) = remaining else {
                    break;
                };
                crate::timer::sleep(remaining).await;
            }
        };
        *task.borrow_mut() = Some((runner.0)(Box::pin(future)));
    });
}
