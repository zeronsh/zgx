//! Web views on iOS: a `WKWebView` standing in a native node's box.
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, Message, class, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, NSObject, NSObjectProtocol},
};
use std::{any::Any, cell::RefCell, rc::Rc};
use zgx::web_view::{ScriptResult, WebView, WebViewBackend, WebViewCommand, WebViewEvent};

#[link(name = "WebKit", kind = "framework")]
unsafe extern "C" {}

fn ns_string(text: &str) -> Retained<AnyObject> {
    crate::ios_system::ns_string(text)
}

fn string_of(text: Option<Retained<AnyObject>>) -> String {
    text.map(|text| {
        // SAFETY: an NSString UIKit gave; the bytes live as long as it, read now.
        unsafe {
            let bytes: *const std::ffi::c_char = msg_send![&*text, UTF8String];
            if bytes.is_null() {
                return String::new();
            }
            std::ffi::CStr::from_ptr(bytes)
                .to_string_lossy()
                .into_owned()
        }
    })
    .unwrap_or_default()
}

/// The page's address now.
fn page_url(view: &AnyObject) -> String {
    // SAFETY: WKWebView's and NSURL's getters, on the main thread.
    let text = unsafe {
        let url: Option<Retained<AnyObject>> = msg_send![view, URL];
        url.and_then(|url| msg_send![&*url, absoluteString])
    };
    string_of(text)
}
fn page_title(view: &AnyObject) -> String {
    // SAFETY: WKWebView's getter, on the main thread.
    string_of(unsafe { msg_send![view, title] })
}

/// Tell the `WebView` what its page now is.
fn report(web: &WebView, view: &AnyObject, loading: bool) {
    // SAFETY: WKWebView's getters, on the main thread.
    let (back, forward): (bool, bool) =
        unsafe { (msg_send![view, canGoBack], msg_send![view, canGoForward]) };
    let (url, title) = (page_url(view), page_title(view));
    web.update_state(|state| {
        state.url = url;
        state.title = title;
        state.loading = loading;
        state.can_go_back = back;
        state.can_go_forward = forward;
    });
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = WebView]
    struct NavigationDelegate;
    unsafe impl NSObjectProtocol for NavigationDelegate {}
    impl NavigationDelegate {
        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn started(&self, view: &AnyObject, _navigation: *mut AnyObject) {
            let web = self.ivars();
            report(web, view, true);
            web.emit(WebViewEvent::NavigationStarted { url: page_url(view) });
        }
        #[unsafe(method(webView:didCommitNavigation:))]
        fn committed(&self, view: &AnyObject, _navigation: *mut AnyObject) {
            let web = self.ivars();
            report(web, view, true);
            web.emit(WebViewEvent::NavigationCommitted { url: page_url(view) });
        }
        #[unsafe(method(webView:didFinishNavigation:))]
        fn finished(&self, view: &AnyObject, _navigation: *mut AnyObject) {
            let web = self.ivars();
            report(web, view, false);
            web.emit(WebViewEvent::TitleChanged { title: page_title(view) });
            web.emit(WebViewEvent::NavigationFinished { url: page_url(view) });
        }
        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn failed(&self, view: &AnyObject, _navigation: *mut AnyObject, error: &AnyObject) {
            self.fail(view, error);
        }
        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn failed_early(&self, view: &AnyObject, _navigation: *mut AnyObject, error: &AnyObject) {
            self.fail(view, error);
        }
        // A title a script sets, as well as a page's own.
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observed(
            &self,
            _path: *mut AnyObject,
            view: &AnyObject,
            _change: *mut AnyObject,
            _context: *mut std::ffi::c_void,
        ) {
            let web = self.ivars();
            report(web, view, false);
            web.emit(WebViewEvent::TitleChanged { title: page_title(view) });
        }
        // A page's `alert`, `confirm` and `prompt`: without these WebKit answers
        // for the person, and a page's question is never seen.
        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn page_alert(
            &self,
            _view: &AnyObject,
            message: &AnyObject,
            _frame: &AnyObject,
            done: &block2::DynBlock<dyn Fn()>,
        ) {
            let done = done.copy();
            page_panel(message, None, false, move |_| done.call(()));
        }
        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn page_confirm(
            &self,
            _view: &AnyObject,
            message: &AnyObject,
            _frame: &AnyObject,
            done: &block2::DynBlock<dyn Fn(Bool)>,
        ) {
            let done = done.copy();
            page_panel(message, None, true, move |answer| {
                done.call((Bool::new(answer.is_some()),));
            });
        }
        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn page_prompt(
            &self,
            _view: &AnyObject,
            prompt: &AnyObject,
            text: Option<&AnyObject>,
            _frame: &AnyObject,
            done: &block2::DynBlock<dyn Fn(*mut AnyObject)>,
        ) {
            let done = done.copy();
            let text = text.map_or_else(|| Some(String::new()), |text| Some(string_of(Some(text.retain()))));
            page_panel(prompt, text, true, move |answer| {
                let answer = answer.map(|answer| crate::ios_system::ns_string(&answer));
                let raw = answer
                    .as_deref()
                    .map_or(std::ptr::null_mut(), |answer| {
                        std::ptr::from_ref(answer).cast_mut()
                    });
                done.call((raw,));
            });
        }
    }
);

/// A page's panel as a system alert. `done` gets `Some` for OK, with the text if
/// `field` gave one, and `None` for Cancel; always, as WebKit requires.
fn page_panel(
    message: &AnyObject,
    field: Option<String>,
    cancels: bool,
    done: impl FnOnce(Option<String>) + 'static,
) {
    let done = Rc::new(std::cell::RefCell::new(Some(done)));
    let Some(root) = crate::ios_system::root_controller() else {
        if let Some(done) = done.borrow_mut().take() {
            done(None);
        }
        return;
    };
    // SAFETY: UIKit's constructors and methods, on the main thread where WebKit
    // asks; the blocks hold what they read.
    unsafe {
        let controller: Retained<AnyObject> = msg_send![class!(UIAlertController),
            alertControllerWithTitle: std::ptr::null::<AnyObject>(),
            message: message,
            preferredStyle: 1isize];
        if let Some(text) = &field {
            let text = crate::ios_system::ns_string(text);
            let configure = block2::RcBlock::new(move |field: *mut AnyObject| {
                let _: () = msg_send![field, setText: &*text];
            });
            let _: () = msg_send![&*controller, addTextFieldWithConfigurationHandler: &*configure];
        }
        let has_field = field.is_some();
        let weak = objc2::rc::Weak::from_retained(&controller);
        let ok_done = done.clone();
        let ok = block2::RcBlock::new(move |_: *mut AnyObject| {
            let text = has_field.then(|| {
                weak.load().map_or_else(String::new, |controller| {
                    let fields: Retained<AnyObject> = msg_send![&*controller, textFields];
                    let first: Option<Retained<AnyObject>> = msg_send![&*fields, firstObject];
                    first.map_or_else(String::new, |first| string_of(msg_send![&*first, text]))
                })
            });
            if let Some(done) = ok_done.borrow_mut().take() {
                done(Some(text.unwrap_or_default()));
            }
            zgx::native_view::wake();
        });
        let title = crate::ios_system::ns_string("OK");
        let action: Retained<AnyObject> = msg_send![class!(UIAlertAction),
            actionWithTitle: &*title, style: 0isize, handler: &*ok];
        let _: () = msg_send![&*controller, addAction: &*action];
        if cancels {
            let cancel = block2::RcBlock::new(move |_: *mut AnyObject| {
                if let Some(done) = done.borrow_mut().take() {
                    done(None);
                }
                zgx::native_view::wake();
            });
            let title = crate::ios_system::ns_string("Cancel");
            let action: Retained<AnyObject> = msg_send![class!(UIAlertAction),
                actionWithTitle: &*title, style: 1isize, handler: &*cancel];
            let _: () = msg_send![&*controller, addAction: &*action];
        }
        present_when_free(root, controller);
    }
}

/// Present `controller` once what `root` is showing has gone: a page asking again
/// as soon as it is answered comes while the last alert is still leaving, UIKit
/// refuses that, and the page would wait for an answer for ever.
fn present_when_free(root: Retained<AnyObject>, controller: Retained<AnyObject>) {
    // SAFETY: UIViewController's getters and method, and NSTimer's constructor, on
    // the main thread, which runs the timer.
    unsafe {
        let busy: Option<Retained<AnyObject>> = msg_send![&*root, presentedViewController];
        if busy.is_none() {
            let _: () = msg_send![&*root,
                presentViewController: &*controller,
                animated: true,
                completion: std::ptr::null::<AnyObject>()];
            return;
        }
        let waiting = RefCell::new(Some((root, controller)));
        let retry = block2::RcBlock::new(move |_: *mut AnyObject| {
            if let Some((root, controller)) = waiting.borrow_mut().take() {
                present_when_free(root, controller);
            }
        });
        let _: Retained<AnyObject> = msg_send![class!(NSTimer),
            scheduledTimerWithTimeInterval: 0.05f64,
            repeats: false,
            block: &*retry];
    }
}

impl NavigationDelegate {
    fn fail(&self, view: &AnyObject, error: &AnyObject) {
        let web = self.ivars();
        report(web, view, false);
        // SAFETY: NSError's getter, on the main thread.
        let reason = string_of(unsafe { msg_send![error, localizedDescription] });
        web.emit(WebViewEvent::NavigationFailed {
            url: page_url(view),
            error: reason,
        });
    }
}

/// A page, held as its `WebView`'s backend.
pub(crate) struct IosWebView {
    view: Retained<AnyObject>,
    _delegate: Retained<NavigationDelegate>,
}

impl IosWebView {
    pub(crate) fn view(&self) -> Retained<AnyObject> {
        self.view.clone()
    }
}

impl Drop for IosWebView {
    fn drop(&mut self) {
        let title = crate::ios_system::ns_string("title");
        // SAFETY: NSObject's method, on the main thread that owns the view, with
        // the observer and key path `backend` added.
        unsafe {
            let _: () = msg_send![&*self.view,
                removeObserver: &*self._delegate,
                forKeyPath: &*title];
        }
    }
}

impl WebViewBackend for IosWebView {
    fn run(&self, command: WebViewCommand) {
        let view = &*self.view;
        // SAFETY: WKWebView's and Foundation's methods, on the main thread where
        // commands run.
        unsafe {
            match command {
                WebViewCommand::LoadUrl(url) => {
                    let text = ns_string(&url);
                    let url: Option<Retained<AnyObject>> =
                        msg_send![class!(NSURL), URLWithString: &*text];
                    if let Some(url) = url {
                        let request: Retained<AnyObject> =
                            msg_send![class!(NSURLRequest), requestWithURL: &*url];
                        let _: Option<Retained<AnyObject>> =
                            msg_send![view, loadRequest: &*request];
                    }
                }
                WebViewCommand::LoadHtml { html, base_url } => {
                    let html = ns_string(&html);
                    let base: Option<Retained<AnyObject>> = base_url.and_then(|base| {
                        let base = ns_string(&base);
                        msg_send![class!(NSURL), URLWithString: &*base]
                    });
                    let _: Option<Retained<AnyObject>> =
                        msg_send![view, loadHTMLString: &*html, baseURL: base.as_deref()];
                }
                WebViewCommand::Reload => {
                    let _: Option<Retained<AnyObject>> = msg_send![view, reload];
                }
                WebViewCommand::Stop => {
                    let _: () = msg_send![view, stopLoading];
                }
                WebViewCommand::GoBack => {
                    let _: Option<Retained<AnyObject>> = msg_send![view, goBack];
                }
                WebViewCommand::GoForward => {
                    let _: Option<Retained<AnyObject>> = msg_send![view, goForward];
                }
                WebViewCommand::Evaluate { script, reply } => {
                    let script = ns_string(&script);
                    let reply = RefCell::new(reply);
                    let done = block2::RcBlock::new(
                        move |result: *mut AnyObject, error: *mut AnyObject| {
                            let Some(reply) = reply.borrow_mut().take() else {
                                return;
                            };
                            let answer: ScriptResult = if error.is_null() {
                                let text: Option<Retained<AnyObject>> = if result.is_null() {
                                    None
                                } else {
                                    msg_send![result, description]
                                };
                                Ok(string_of(text))
                            } else {
                                Err(string_of(msg_send![error, localizedDescription]))
                            };
                            reply(answer);
                        },
                    );
                    let _: () =
                        msg_send![view, evaluateJavaScript: &*script, completionHandler: &*done];
                }
                _ => {}
            }
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `web`'s page, made and attached the first time it is asked for.
pub(crate) fn backend(web: &WebView) -> Option<Rc<dyn WebViewBackend>> {
    if let Some(backend) = web.backend() {
        return Some(backend);
    }
    let mtm = MainThreadMarker::new()?;
    let delegate = NavigationDelegate::alloc(mtm).set_ivars(web.clone());
    // SAFETY: NSObject's `init`; WebKit's constructors and setters, on the main
    // thread. The view holds its delegate weakly: `IosWebView` keeps it.
    let (view, delegate) = unsafe {
        let delegate: Retained<NavigationDelegate> = msg_send![super(delegate), init];
        let config: Retained<AnyObject> = msg_send![class!(WKWebViewConfiguration), new];
        let frame = objc2_core_foundation::CGRect::default();
        let view: Retained<AnyObject> = msg_send![msg_send![class!(WKWebView), alloc],
            initWithFrame: frame,
            configuration: &*config];
        let _: () = msg_send![&*view, setNavigationDelegate: &*delegate];
        let _: () = msg_send![&*view, setUIDelegate: &*delegate];
        let title = crate::ios_system::ns_string("title");
        let _: () = msg_send![&*view,
            addObserver: &*delegate,
            forKeyPath: &*title,
            options: 1usize,
            context: std::ptr::null_mut::<std::ffi::c_void>()];
        let _: () = msg_send![&*view, setAllowsBackForwardNavigationGestures: true];
        (view, delegate)
    };
    let backend: Rc<dyn WebViewBackend> = Rc::new(IosWebView {
        view,
        _delegate: delegate,
    });
    web.attach(backend.clone());
    Some(backend)
}
