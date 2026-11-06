//! Web views on iOS: a `WKWebView` standing in a native node's box.
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol},
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
    }
);

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
