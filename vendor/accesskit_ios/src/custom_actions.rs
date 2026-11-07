// zgx patch: what a host offers VoiceOver's actions rotor for a node, asked for
// only as VoiceOver reaches it.
use accesskit::NodeId;
use objc2::{
    declare_class, msg_send, msg_send_id, mutability, rc::Retained as Id, runtime::AnyObject, sel,
    ClassType, DeclaredClass,
};
use objc2_foundation::{MainThreadMarker, NSObject, NSString};
use std::cell::RefCell;
use std::rc::Rc;

/// An entry of the rotor.
pub struct CustomAction {
    /// What VoiceOver says.
    pub name: String,
    /// What choosing it does.
    pub run: Rc<dyn Fn()>,
}
type Provider = Box<dyn Fn(NodeId) -> Vec<CustomAction>>;
thread_local! {
    static PROVIDER: RefCell<Option<Provider>> = const { RefCell::new(None) };
    /// The targets of the actions last given: UIKit holds its targets weakly.
    static TARGETS: RefCell<Vec<Id<Target>>> = const { RefCell::new(Vec::new()) };
}
/// Say what each node offers.
pub fn set_custom_actions(provider: impl Fn(NodeId) -> Vec<CustomAction> + 'static) {
    PROVIDER.with(|slot| *slot.borrow_mut() = Some(Box::new(provider)));
}

struct TargetIvars {
    run: Rc<dyn Fn()>,
}
declare_class!(
    struct Target;
    unsafe impl ClassType for Target {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "ZgxCustomActionTarget";
    }
    impl DeclaredClass for Target {
        type Ivars = TargetIvars;
    }
    unsafe impl Target {
        #[method(perform:)]
        fn perform(&self, _action: *mut AnyObject) -> bool {
            (self.ivars().run)();
            true
        }
    }
);

pub(crate) fn actions_for(node: NodeId) -> *mut AnyObject {
    let actions = PROVIDER.with(|slot| slot.borrow().as_ref().map_or(Vec::new(), |provider| provider(node)));
    let Some(mtm) = MainThreadMarker::new() else {
        return std::ptr::null_mut();
    };
    if actions.is_empty() {
        return std::ptr::null_mut();
    }
    // SAFETY: Foundation's and UIKit's constructors, on the main thread; the
    // array is handed over autoreleased.
    unsafe {
        let list: Id<AnyObject> = msg_send_id![objc2::class!(NSMutableArray), array];
        for action in actions {
            let target = mtm.alloc::<Target>().set_ivars(TargetIvars { run: action.run });
            let target: Id<Target> = msg_send_id![super(target), init];
            let name = NSString::from_str(&action.name);
            let made: Id<AnyObject> = msg_send_id![
                msg_send_id![objc2::class!(UIAccessibilityCustomAction), alloc],
                initWithName: &*name,
                target: &*target,
                selector: sel!(perform:)
            ];
            let _: () = msg_send![&*list, addObject: &*made];
            TARGETS.with(|targets| {
                let mut targets = targets.borrow_mut();
                if targets.len() > 64 {
                    targets.remove(0);
                }
                targets.push(target);
            });
        }
        Id::autorelease_return(list)
    }
}
