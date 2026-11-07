//! zgx patch: that the adapter lets go when assistive technology stops, in a
//! simulator's real UIKit. A failure aborts; "ok" lines are what passed.
use accesskit::{
    Action, ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, Node, NodeId,
    Role, Tree, TreeId, TreeUpdate,
};
use accesskit_ios::{Adapter, expire_read_lease};
use objc2::{class, msg_send_id, rc::Retained};
use objc2_foundation::NSNotificationCenter;
use objc2_ui_kit::{UIAccessibilityIsVoiceOverRunning, UIAccessibilityVoiceOverStatusDidChangeNotification, UIView};
use std::{cell::Cell, ffi::c_void, rc::Rc};

#[derive(Clone, Default)]
struct Counts {
    asked: Rc<Cell<u32>>,
    gone: Rc<Cell<u32>>,
}
fn tree() -> TreeUpdate {
    let mut root = Node::new(Role::Window);
    root.add_action(Action::Focus);
    TreeUpdate {
        nodes: vec![(NodeId(1), root)],
        tree: Some(Tree::new(NodeId(1))),
        tree_id: TreeId::ROOT,
        focus: NodeId(1),
    }
}
impl ActivationHandler for Counts {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.asked.set(self.asked.get() + 1);
        Some(tree())
    }
}
impl ActionHandler for Counts {
    fn do_action(&mut self, _: ActionRequest) {}
}
impl DeactivationHandler for Counts {
    fn deactivate_accessibility(&mut self) {
        self.gone.set(self.gone.get() + 1);
    }
}
fn adapter(counts: &Counts) -> (Retained<UIView>, Adapter) {
    let view: Retained<UIView> = unsafe { msg_send_id![class!(UIView), new] };
    let raw = Retained::as_ptr(&view) as *mut c_void;
    let adapter = unsafe { Adapter::new(raw, counts.clone(), counts.clone(), counts.clone()) };
    (view, adapter)
}
/// How many updates a frame's change makes: 0 is what "shut down" costs.
fn updates(adapter: &Adapter) -> u32 {
    let made = Cell::new(0);
    let _ = adapter.update_if_active(|| {
        made.set(made.get() + 1);
        tree()
    });
    made.get()
}

fn nothing_is_built_until_a_tree_is_read() {
    let counts = Counts::default();
    let (_view, adapter) = adapter(&counts);
    assert!(!adapter.is_accessibility_element());
    assert_eq!(counts.asked.get(), 0, "asking if it is an element builds nothing");
    assert_eq!(updates(&adapter), 0);
    println!("ok: nothing is built until a tree is read");
}

fn a_stop_notification_shuts_it_down() {
    let counts = Counts::default();
    let (_view, adapter) = adapter(&counts);
    let _ = adapter.accessibility_elements();
    assert_eq!(counts.asked.get(), 1);
    assert_eq!(updates(&adapter), 1, "while read, updates are made");
    // What iOS posts when VoiceOver stops; nothing runs here, as then.
    unsafe {
        assert!(!UIAccessibilityIsVoiceOverRunning().as_bool());
        NSNotificationCenter::defaultCenter().postNotificationName_object(
            UIAccessibilityVoiceOverStatusDidChangeNotification,
            None,
        );
    }
    assert_eq!(counts.gone.get(), 1, "the host is told");
    assert_eq!(updates(&adapter), 0, "no update is even made");
    // It comes back for the next reader.
    let _ = adapter.accessibility_elements();
    assert_eq!(counts.asked.get(), 2, "asked again");
    assert_eq!(updates(&adapter), 1);
    println!("ok: a stop notification shuts it down, and it comes back when read");
}

fn a_read_with_nothing_running_lapses() {
    let counts = Counts::default();
    let (_view, adapter) = adapter(&counts);
    let _ = adapter.accessibility_elements();
    assert_eq!(updates(&adapter), 1, "within the lease");
    expire_read_lease();
    assert_eq!(updates(&adapter), 0, "no notification, and it lets go");
    assert_eq!(counts.gone.get(), 1, "the host is told");
    println!("ok: a read with nothing running lapses");
}

fn main() {
    nothing_is_built_until_a_tree_is_read();
    a_stop_notification_shuts_it_down();
    a_read_with_nothing_running_lapses();
}
