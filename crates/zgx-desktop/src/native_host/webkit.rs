//! A `WKWebView` behind a `zgx::web_view::WebView` controller.
//!
//! WebKit's callbacks (navigation delegate, key-value observation, script
//! replies, page messages) arrive on the main thread but can arrive inside
//! any zgx call into WebKit; each reads what it needs and posts the state
//! change (`zgx::native_view::post`), applied at the event loop's next safe
//! point. The page's view is mounted by the window host (macos.rs).
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class,
    msg_send,
    rc::{Retained, Weak},
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject},
    sel,
};
use objc2_app_kit::{NSColor, NSView};
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSError, NSHTTPURLResponse, NSKeyValueChangeKey,
    NSKeyValueObservingOptions, NSNumber, NSObjectNSKeyValueObserverRegistration, NSRect, NSString,
    NSURL, NSURLRequest, NSUUID,
};
use objc2_web_kit::{
    WKDownload, WKDownloadDelegate, WKFrameInfo, WKNavigation, WKNavigationAction,
    WKNavigationActionPolicy, WKNavigationDelegate, WKNavigationResponse,
    WKNavigationResponsePolicy, WKNavigationType, WKOpenPanelParameters, WKScriptMessage,
    WKScriptMessageHandler, WKUIDelegate, WKURLSchemeHandler, WKURLSchemeTask,
    WKUserContentController, WKUserScript, WKUserScriptInjectionTime, WKWebView,
    WKWebViewConfiguration, WKWebsiteDataStore, WKWindowFeatures,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::c_void,
    mem::ManuallyDrop,
    rc::Rc,
};
use zgx::web_view::{
    NavigationKind, NavigationRequest, SchemeHandler, SchemeRequest, SchemeResponder,
    SchemeResponse, ScriptResult, WeakWebView, WebProfile, WebView, WebViewBackend, WebViewCommand,
    WebViewEvent,
};

// SAFETY: declares nothing; it only links WebKit, whose classes objc2-web-kit names.
#[link(name = "WebKit", kind = "framework")]
unsafe extern "C" {}

/// Observed for state: navigation, title, loading and history.
const OBSERVED: [&str; 6] = [
    "URL",
    "title",
    "loading",
    "estimatedProgress",
    "canGoBack",
    "canGoForward",
];
/// The print panel for the page, a sheet on its window (`WebView::print`,
/// the page's `window.print()`); nothing for a page in no window.
fn print_page(view: &WKWebView) {
    let Some(window) = view.window() else {
        return;
    };
    // SAFETY: WKWebView's print operation (macOS 11) and AppKit's print
    // calls on the main thread; no delegate, so no selector is called back.
    unsafe {
        let info = objc2_app_kit::NSPrintInfo::sharedPrintInfo();
        let operation = view.printOperationWithPrintInfo(&info);
        operation.setShowsPrintPanel(true);
        operation.setShowsProgressPanel(true);
        // WebKit's print view comes with no size, and prints blank without
        // one: the page's own.
        if let Some(printed) = operation.view() {
            printed.setFrame(view.bounds());
        }
        operation.runOperationModalForWindow_delegate_didRunSelector_contextInfo(
            &window,
            None,
            None,
            std::ptr::null_mut(),
        );
    }
}
/// A page's script dialog (`alert`, `confirm`, `prompt` with its default).
enum Dialog {
    Alert,
    Confirm,
    Prompt(String),
}
/// A page's script dialog as an alert sheet on its window, the message its
/// text and the page's origin under it (as Safari says which page asks):
/// `answer` gets `Some` for OK (a prompt's field, else empty), `None` for
/// Cancel, or at once if the page is in no window. WebKit waits for it,
/// the page's script paused, as a browser's dialog pauses it.
fn page_dialog(
    view: &WKWebView,
    frame: &WKFrameInfo,
    message: &NSString,
    dialog: Dialog,
    answer: impl FnOnce(Option<String>) + 'static,
) {
    use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSModalResponse, NSTextField};
    let mtm = MainThreadMarker::from(view);
    let Some(window) = view.window() else {
        // No window to show it on: an alert is dismissed, the rest cancelled.
        return answer(matches!(dialog, Dialog::Alert).then(String::new));
    };
    // SAFETY: getters of the frame WebKit passes for this call.
    let origin = unsafe {
        let origin = frame.securityOrigin();
        let (scheme, host) = (origin.protocol().to_string(), origin.host().to_string());
        if host.is_empty() {
            String::new()
        } else {
            format!("{scheme}://{host}")
        }
    };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(message);
    alert.setInformativeText(&NSString::from_str(&origin));
    alert.addButtonWithTitle(&NSString::from_str("OK"));
    if !matches!(dialog, Dialog::Alert) {
        alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    }
    let field = match &dialog {
        Dialog::Prompt(default) => {
            let field = NSTextField::initWithFrame(
                NSTextField::alloc(mtm),
                NSRect::new(
                    objc2_foundation::NSPoint::new(0., 0.),
                    objc2_foundation::NSSize::new(260., 24.),
                ),
            );
            field.setStringValue(&NSString::from_str(default));
            alert.setAccessoryView(Some(&field));
            alert.window().setInitialFirstResponder(Some(&field));
            Some(field)
        }
        _ => None,
    };
    let answer = Cell::new(Some(answer));
    let done = block2::RcBlock::new(move |response: NSModalResponse| {
        let Some(answer) = answer.take() else { return };
        let ok = response == NSAlertFirstButtonReturn;
        answer(ok.then(|| {
            field
                .as_ref()
                .map(|field| field.stringValue().to_string())
                .unwrap_or_default()
        }));
    });
    alert.beginSheetModalForWindow_completionHandler(&window, Some(&done));
}
/// `fullscreenState`, observed where WKWebView has it (macOS 13).
fn full_screen_key(view: &WKWebView) -> Option<&'static &'static str> {
    // SAFETY: `respondsToSelector:` on the main thread.
    let has: bool = unsafe { msg_send![view, respondsToSelector: sel!(fullscreenState)] };
    has.then_some(&"fullscreenState")
}
/// An element of the page is full screen, or going to be.
fn full_screen(view: &WKWebView) -> bool {
    if full_screen_key(view).is_none() {
        return false;
    }
    // SAFETY: WKWebView's `fullscreenState` (an NSInteger) where it has it.
    let state: isize = unsafe { msg_send![view, fullscreenState] };
    // Entering or in, not exiting or out.
    state == 1 || state == 2
}
/// The pages back and forward from the current one, nearest first.
fn history(
    view: &WKWebView,
) -> (
    Vec<zgx::web_view::HistoryItem>,
    Vec<zgx::web_view::HistoryItem>,
) {
    // SAFETY (the three below): WebKit getters on the main thread (a
    // `WKWebView` is main-thread only); `itemAtIndex:` returns nil past
    // either end, and each result is retained.
    let list = unsafe { view.backForwardList() };
    let walk = |step: isize| {
        (1..)
            .map_while(|n| unsafe { list.itemAtIndex(n * step) })
            .take(50)
            .map(|item| unsafe {
                zgx::web_view::HistoryItem {
                    url: item
                        .URL()
                        .absoluteString()
                        .map(|url| url.to_string())
                        .unwrap_or_default(),
                    title: item
                        .title()
                        .map(|title| title.to_string())
                        .unwrap_or_default(),
                }
            })
            .collect()
    };
    (walk(-1), walk(1))
}

/// The page-side name of the app's message channel.
const CHANNEL: &str = "zgx";
/// The page's `window.print()`, posted here (`PRINT_SCRIPT`): WKWebView's
/// own print call for it is unpublished.
const PRINT: &str = "zgxPrint";
const PRINT_SCRIPT: &str = "(() => { const handler = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.zgxPrint; if (handler) window.print = () => handler.postMessage(null); })();";
/// Defines `window.zgx.postMessage(value)`, sending `value` as JSON.
const CHANNEL_SCRIPT: &str = "(() => { const handler = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.zgx; if (!handler || window.zgx) return; Object.defineProperty(window, 'zgx', { value: Object.freeze({ postMessage(value) { handler.postMessage(JSON.stringify(value === undefined ? null : value)); } }) }); Object.defineProperty(window, 'zgui', { value: window.zgx }); })();"; // rename-to-zgx: keep

// SAFETY: a WKWebView subclass, main-thread only as views are, adding no
// state; `performKeyEquivalent:` and `keyDown:` have NSResponder's
// signatures and call WKWebView's own for every key they do not take.
define_class!(
    /// The page view: WKWebView as it is, configured as the app asks, but
    /// for the shortcuts the app keeps whatever has the keyboard
    /// (`KeyBinding::reserved`), which it hands the app before WebKit has
    /// them. Every other key is WebKit's, then the app's if the page leaves
    /// it alone.
    #[unsafe(super(WKWebView, NSView, objc2_app_kit::NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ZgxWebView"]
    #[ivars = ()]
    pub(crate) struct ZgxWebView;
    impl ZgxWebView {
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &objc2_app_kit::NSEvent) -> bool {
            // SAFETY: WKWebView's own, with the same event.
            super::macos::take_reserved_key(self, event)
                || unsafe { msg_send![super(self), performKeyEquivalent: event] }
        }
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &objc2_app_kit::NSEvent) {
            if !super::macos::take_reserved_key(self, event) {
                // SAFETY: WKWebView's own, with the same event.
                let _: () = unsafe { msg_send![super(self), keyDown: event] };
            }
        }
    }
);

pub(crate) struct ObserverState {
    web: WeakWebView,
    view: RefCell<Weak<WKWebView>>,
    new_windows: bool,
    /// The page's downloads the app is saving: each download, its
    /// progress (observed), and what hears how it goes.
    downloads: RefCell<Vec<Saving>>,
}
/// A download the app saves.
pub(crate) struct Saving {
    download: Retained<WKDownload>,
    progress: Retained<objc2_foundation::NSProgress>,
    status: zgx::web_view::DownloadStatusFn,
}

// SAFETY: an NSObject subclass, main-thread only (WebKit calls its delegates
// and KVO observers there), with nothing to clean up beyond its ivars. Each
// method's signature is the Objective-C one: KVO's
// `observeValueForKeyPath:ofObject:change:context:`, the navigation
// delegate's (the decision handler a `void (^)(WKNavigationActionPolicy)`
// block, called once), the UI delegate's `createWebView…` returning an
// autoreleased web view or nil (`method_id`), and the script message
// handler's. WebKit holds it unretained as a delegate and an observer;
// `MacWebView` keeps it alive and unregisters it in its `Drop`.
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ZgxWebObserver"]
    #[ivars = ObserverState]
    pub(crate) struct Observer;
    unsafe impl NSObjectProtocol for Observer {}
    impl Observer {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(&self, key: Option<&NSString>, object: Option<&AnyObject>, _change: Option<&NSDictionary<NSKeyValueChangeKey, AnyObject>>, _context: *mut c_void) {
            // A saved download's progress (WebKit updates it on the main
            // thread, as its messages come).
            if key.is_some_and(|key| key.to_string() == "completedUnitCount") {
                let mut downloads = self.ivars().downloads.borrow_mut();
                if let Some(saving) = downloads.iter_mut().find(|saving| {
                    object.is_some_and(|object| std::ptr::eq(object, Retained::as_ptr(&saving.progress).cast()))
                }) {
                    let (received, total) = (saving.progress.completedUnitCount(), saving.progress.totalUnitCount());
                    (saving.status)(zgx::web_view::DownloadStatus::Progress {
                        received: received.max(0) as u64,
                        total: (total > 0).then_some(total as u64),
                    });
                }
                return;
            }
            // Progress and loading change many times a load; history does not.
            let history = key.is_none_or(|key| {
                let key = key.to_string();
                key != "estimatedProgress" && key != "loading"
            });
            self.changed_with(history);
        }
    }
    unsafe impl WKNavigationDelegate for Observer {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn policy(&self, _view: &WKWebView, action: &WKNavigationAction, decision: &block2::DynBlock<dyn Fn(WKNavigationActionPolicy)>) {
            // SAFETY: getters of the action WebKit passes for this call (main
            // thread), each result retained or nil-checked.
            let request = unsafe {
                NavigationRequest {
                    url: action.request().URL().and_then(|url| url.absoluteString()).map(|url| url.to_string()).unwrap_or_default(),
                    // No target frame: a new window, which the UI delegate handles.
                    main_frame: action.targetFrame().is_none_or(|frame| frame.isMainFrame()),
                    kind: match action.navigationType() {
                        WKNavigationType::LinkActivated => NavigationKind::Link,
                        WKNavigationType::FormSubmitted => NavigationKind::FormSubmission,
                        WKNavigationType::BackForward => NavigationKind::BackForward,
                        WKNavigationType::Reload => NavigationKind::Reload,
                        WKNavigationType::FormResubmitted => NavigationKind::FormResubmission,
                        _ => NavigationKind::Other,
                    },
                    source_url: action
                        .sourceFrame()
                        .and_then(|frame| frame.request().URL())
                        .and_then(|url| url.absoluteString())
                        .map(|url| url.to_string())
                        .unwrap_or_default(),
                }
            };
            let allowed = self.ivars().web.upgrade().is_none_or(|web| web.allows(&request));
            // A `download` link becomes a download (macOS 11.3), which the
            // app saves or not (`WKDownloadDelegate` below).
            // SAFETY: `respondsToSelector:`, then the getter where it exists.
            let download = unsafe {
                let has: bool = msg_send![action, respondsToSelector: sel!(shouldPerformDownload)];
                has && action.shouldPerformDownload()
            };
            decision.call((match (allowed, download) {
                (false, _) => WKNavigationActionPolicy::Cancel,
                (true, true) => WKNavigationActionPolicy::Download,
                (true, false) => WKNavigationActionPolicy::Allow,
            },));
        }
        /// A response the page can't show, or an attachment: a download.
        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn response_policy(&self, _view: &WKWebView, response: &WKNavigationResponse, decision: &block2::DynBlock<dyn Fn(WKNavigationResponsePolicy)>) {
            // SAFETY: getters of the response WebKit passes for this call.
            let download = unsafe {
                let attachment = response
                    .response()
                    .downcast::<NSHTTPURLResponse>()
                    .ok()
                    .and_then(|http| http.valueForHTTPHeaderField(&NSString::from_str("Content-Disposition")))
                    .is_some_and(|value| value.to_string().trim_start().to_ascii_lowercase().starts_with("attachment"));
                !response.canShowMIMEType() || attachment
            };
            decision.call((if download {
                WKNavigationResponsePolicy::Download
            } else {
                WKNavigationResponsePolicy::Allow
            },));
        }
        #[unsafe(method(webView:navigationAction:didBecomeDownload:))]
        fn action_download(&self, _view: &WKWebView, _action: &WKNavigationAction, download: &WKDownload) {
            // SAFETY: the download WebKit passes; it holds its delegate unretained, and `MacWebView` keeps the observer.
            unsafe { download.setDelegate(Some(ProtocolObject::from_ref(self))) };
        }
        #[unsafe(method(webView:navigationResponse:didBecomeDownload:))]
        fn response_download(&self, _view: &WKWebView, _response: &WKNavigationResponse, download: &WKDownload) {
            // SAFETY: as `action_download`.
            unsafe { download.setDelegate(Some(ProtocolObject::from_ref(self))) };
        }
        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn start(&self, view: &WKWebView, _navigation: Option<&WKNavigation>) {
            let url = url_of(view);
            self.post(move |web| {
                web.update_state(|state| state.error = None);
                web.emit(WebViewEvent::NavigationStarted { url });
            });
            self.changed();
        }
        #[unsafe(method(webView:didCommitNavigation:))]
        fn commit(&self, view: &WKWebView, _navigation: Option<&WKNavigation>) {
            let url = url_of(view);
            self.post(move |web| web.emit(WebViewEvent::NavigationCommitted { url }));
            self.changed();
        }
        #[unsafe(method(webView:didFinishNavigation:))]
        fn finish(&self, view: &WKWebView, _navigation: Option<&WKNavigation>) {
            self.changed();
            let url = url_of(view);
            self.post(move |web| web.emit(WebViewEvent::NavigationFinished { url }));
        }
        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn provisional_error(&self, view: &WKWebView, _navigation: Option<&WKNavigation>, error: &NSError) {
            self.failed(view, error);
        }
        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn navigation_error(&self, view: &WKWebView, _navigation: Option<&WKNavigation>, error: &NSError) {
            self.failed(view, error);
        }
        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn terminated(&self, _view: &WKWebView) {
            self.post(|web| {
                web.update_state(|state| {
                    state.loading = false;
                    state.error = Some("The page stopped responding.".into());
                });
                web.emit(WebViewEvent::ProcessTerminated);
            });
        }
    }
    unsafe impl WKUIDelegate for Observer {
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_window(&self, _view: &WKWebView, _configuration: &WKWebViewConfiguration, action: &WKNavigationAction, _features: &WKWindowFeatures) -> Option<Retained<WKWebView>> {
            if self.ivars().new_windows {
                // SAFETY: getters of the action WebKit passes for this call.
                let url = unsafe { action.request().URL() }
                    .and_then(|url| url.absoluteString())
                    .map(|url| url.to_string())
                    .unwrap_or_default();
                self.post(move |web| web.emit(WebViewEvent::NewWindowRequested { url }));
            }
            None
        }
        /// `alert(message)`: an alert sheet with OK.
        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn alert(&self, view: &WKWebView, message: &NSString, frame: &WKFrameInfo, completion: &block2::DynBlock<dyn Fn()>) {
            let completion = completion.copy();
            page_dialog(view, frame, message, Dialog::Alert, move |_| completion.call(()));
        }
        /// `confirm(message)`: OK (true) or Cancel (false).
        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn confirm(&self, view: &WKWebView, message: &NSString, frame: &WKFrameInfo, completion: &block2::DynBlock<dyn Fn(objc2::runtime::Bool)>) {
            let completion = completion.copy();
            page_dialog(view, frame, message, Dialog::Confirm, move |answer| {
                completion.call((objc2::runtime::Bool::new(answer.is_some()),))
            });
        }
        /// `prompt(message, default)`: the field's text for OK, null for Cancel.
        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn prompt(&self, view: &WKWebView, message: &NSString, default: Option<&NSString>, frame: &WKFrameInfo, completion: &block2::DynBlock<dyn Fn(*mut NSString)>) {
            let completion = completion.copy();
            let default = default.map(|text| text.to_string()).unwrap_or_default();
            page_dialog(view, frame, message, Dialog::Prompt(default), move |answer| {
                let text = answer.map(|text| NSString::from_str(&text));
                completion.call((text.as_ref().map_or(std::ptr::null_mut(), |text| Retained::as_ptr(text).cast_mut()),))
            });
        }
        /// A file input (`<input type=file>`): the window's own file dialog
        /// as a sheet (`WebView::choose_files`), its answer the page's.
        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        fn open_panel(&self, _view: &WKWebView, parameters: &WKOpenPanelParameters, _frame: &WKFrameInfo, completion: &block2::DynBlock<dyn Fn(*mut NSArray<NSURL>)>) {
            // SAFETY: getters of the parameters WebKit passes for this call.
            let (multiple, folder) = unsafe { (parameters.allowsMultipleSelection(), parameters.allowsDirectories()) };
            let completion = completion.copy();
            let answer = move |files: Option<Vec<std::path::PathBuf>>| {
                let urls = files.map(|files| {
                    let urls: Vec<Retained<NSURL>> = files
                        .iter()
                        .filter_map(|file| Some(NSURL::fileURLWithPath(&NSString::from_str(file.to_str()?))))
                        .collect();
                    NSArray::from_retained_slice(&urls)
                });
                completion.call((urls.as_ref().map_or(std::ptr::null_mut(), |urls| Retained::as_ptr(urls).cast_mut()),));
            };
            match self.ivars().web.upgrade() {
                Some(web) => web.choose_files(multiple, folder, answer),
                None => answer(None),
            }
        }
    }
    unsafe impl WKDownloadDelegate for Observer {
        /// Where to save it: the app's answer (`WebView::download`), now or
        /// later; nil cancels it, as with no handler.
        #[unsafe(method(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:))]
        fn destination(&self, download: &WKDownload, response: &objc2_foundation::NSURLResponse, name: &NSString, completion: &block2::DynBlock<dyn Fn(*mut NSURL)>) {
            let completion = completion.copy();
            // SAFETY: getters of the download and response WebKit passes.
            let request = unsafe {
                zgx::web_view::DownloadRequest::new(
                    download
                        .originalRequest()
                        .and_then(|request| request.URL())
                        .and_then(|url| url.absoluteString())
                        .map(|url| url.to_string())
                        .unwrap_or_default(),
                    name.to_string(),
                    response.MIMEType().map(|mime| mime.to_string()).unwrap_or_default(),
                )
            };
            let Some(web) = self.ivars().web.upgrade() else {
                completion.call((std::ptr::null_mut(),));
                return;
            };
            let observer = self.retain();
            let download = download.retain();
            web.download(
                request,
                zgx::web_view::DownloadResponder::new(move |answer| {
                    let Some((path, status)) = answer else {
                        completion.call((std::ptr::null_mut(),));
                        return;
                    };
                    // SAFETY: the download's progress, observed by the
                    // observer, which stops before it lets the download go.
                    unsafe {
                        let progress = objc2_foundation::NSProgressReporting::progress(&*download);
                        progress.addObserver_forKeyPath_options_context(
                            &observer,
                            &NSString::from_str("completedUnitCount"),
                            NSKeyValueObservingOptions::empty(),
                            std::ptr::null_mut(),
                        );
                        observer.ivars().downloads.borrow_mut().push(Saving { download, progress, status });
                    }
                    let url = path.to_str().map(|path| NSURL::fileURLWithPath(&NSString::from_str(path)));
                    completion.call((url.as_ref().map_or(std::ptr::null_mut(), |url| Retained::as_ptr(url).cast_mut()),));
                }),
            );
        }
        #[unsafe(method(downloadDidFinish:))]
        fn download_finished(&self, download: &WKDownload) {
            if let Some(mut saving) = self.saved(download) {
                (saving.status)(zgx::web_view::DownloadStatus::Finished);
            }
        }
        #[unsafe(method(download:didFailWithError:resumeData:))]
        fn download_failed(&self, download: &WKDownload, error: &NSError, _resume: Option<&NSData>) {
            if let Some(mut saving) = self.saved(download) {
                (saving.status)(zgx::web_view::DownloadStatus::Failed(error.localizedDescription().to_string()));
            }
        }
    }
    unsafe impl WKScriptMessageHandler for Observer {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn message(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            // SAFETY: the message's name, on the main thread.
            if unsafe { message.name() }.to_string() == PRINT {
                if let Some(view) = self.ivars().view.borrow().load() {
                    print_page(&view);
                }
                return;
            }
            // SAFETY: getters of the message WebKit passes for this call
            // (main thread); the body is any of the JavaScript-bridged
            // Foundation objects, which `json` reads as `AnyObject`.
            let body = unsafe { message.body() };
            let body = match body.downcast_ref::<NSString>() {
                Some(text) => text.to_string(),
                None => json(Some(&body)),
            };
            // Who sent it: any page, any frame, can.
            // SAFETY: as above: getters on the message, each result retained.
            let (origin, main_frame) = unsafe {
                let frame = message.frameInfo();
                let origin = frame.securityOrigin();
                let (scheme, host, port) =
                    (origin.protocol().to_string(), origin.host().to_string(), origin.port());
                let origin = if port == 0 {
                    format!("{scheme}://{host}")
                } else {
                    format!("{scheme}://{host}:{port}")
                };
                (origin, frame.isMainFrame())
            };
            self.post(move |web| {
                web.emit(WebViewEvent::Message {
                    body,
                    origin,
                    main_frame,
                })
            });
        }
    }
);

pub(crate) struct SchemeState {
    web_view: u64,
    serve: Rc<dyn Fn(SchemeRequest, SchemeResponder)>,
}

// SAFETY: an NSObject subclass, main-thread only (WebKit calls scheme
// handlers on the main thread), with nothing to clean up beyond its ivars;
// its methods have `WKURLSchemeHandler`'s signatures. The configuration
// retains it; `MacWebView` keeps it too.
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ZgxSchemeHandler"]
    #[ivars = SchemeState]
    pub(crate) struct SchemeServer;
    unsafe impl NSObjectProtocol for SchemeServer {}
    unsafe impl WKURLSchemeHandler for SchemeServer {
        #[unsafe(method(webView:startURLSchemeTask:))]
        fn start(&self, _view: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            // SAFETY: the task's request, retained; a getter with no precondition.
            let request = unsafe { task.request() };
            let request = {
                SchemeRequest {
                    web_view: self.ivars().web_view,
                    url: request
                        .URL()
                        .and_then(|url| url.absoluteString())
                        .map(|url| url.to_string())
                        .unwrap_or_default(),
                    method: request
                        .HTTPMethod()
                        .map(|method| method.to_string())
                        .unwrap_or_else(|| "GET".into()),
                    headers: request
                        .allHTTPHeaderFields()
                        .map(|fields| {
                            let (keys, values) = fields.to_vecs();
                            keys.iter()
                                .zip(values.iter())
                                .map(|(k, v)| (k.to_string(), v.to_string()))
                                .collect()
                        })
                        .unwrap_or_default(),
                    body: request
                        .HTTPBody()
                        .map(|body| body.to_vec())
                        .unwrap_or_default(),
                    main_document_url: request
                        .mainDocumentURL()
                        .and_then(|url| url.absoluteString())
                        .map(|url| url.to_string())
                        .unwrap_or_default(),
                }
            };
            let id = SCHEME_TASKS.with(|tasks| {
                let id = NEXT_SCHEME_TASK.with(|next| {
                    next.set(next.get() + 1);
                    next.get()
                });
                tasks.borrow_mut().insert(id, task.retain());
                id
            });
            let responder = SchemeResponder::new(move |response| {
                dispatch2::DispatchQueue::main()
                    .exec_async(move || finish_scheme_task(id, response));
            });
            (self.ivars().serve)(request, responder);
        }
        #[unsafe(method(webView:stopURLSchemeTask:))]
        fn stop(&self, _view: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            // WebKit gave up on it (navigation, close): a late answer is dropped.
            SCHEME_TASKS.with(|tasks| {
                tasks
                    .borrow_mut()
                    .retain(|_, kept| !std::ptr::eq(&**kept, task))
            });
        }
    }
);

thread_local! {
    /// Requests being served, by id, until answered or stopped (main thread).
    static SCHEME_TASKS: RefCell<HashMap<u64, Retained<ProtocolObject<dyn WKURLSchemeTask>>>> = RefCell::new(HashMap::new());
    static NEXT_SCHEME_TASK: Cell<u64> = const { Cell::new(0) };
}

impl SchemeServer {
    fn new(web: &WebView, handler: &SchemeHandler, mtm: MainThreadMarker) -> Retained<Self> {
        let object = mtm.alloc().set_ivars(SchemeState {
            web_view: web.id(),
            serve: handler.serve.clone(),
        });
        // SAFETY: NSObject's `init`, on a fresh allocation with its ivars set.
        unsafe { msg_send![super(object), init] }
    }
}

/// Hands a response to WebKit (main thread), unless the request was stopped.
fn finish_scheme_task(id: u64, response: Option<SchemeResponse>) {
    let Some(task) = SCHEME_TASKS.with(|tasks| tasks.borrow_mut().remove(&id)) else {
        return;
    };
    // SAFETY: on the main thread (the main queue runs this), as WebKit
    // requires. A task is answered once, and only while WebKit still wants
    // it: it leaves `SCHEME_TASKS` here or when WebKit stops it, and a
    // stopped task is never answered (WebKit raises on either misuse).
    unsafe {
        let Some(url) = task.request().URL() else {
            // Nothing to answer for: fail it rather than leave it waiting.
            task.didFailWithError(&NSError::errorWithDomain_code_userInfo(
                &NSString::from_str("NSURLErrorDomain"),
                -1000,
                None,
            ));
            return;
        };
        let Some(response) = response else {
            // Unanswered: fail it as a network error for that URL would.
            let domain = NSString::from_str("NSURLErrorDomain");
            let key = NSString::from_str("NSErrorFailingURLKey");
            let info = NSDictionary::<NSString, AnyObject>::from_slices(&[&*key], &[&**url]);
            task.didFailWithError(&NSError::errorWithDomain_code_userInfo(
                &domain,
                -1,
                Some(&info),
            ));
            return;
        };
        let keys: Vec<Retained<NSString>> = response
            .headers
            .iter()
            .map(|(k, _)| NSString::from_str(k))
            .collect();
        let values: Vec<Retained<NSString>> = response
            .headers
            .iter()
            .map(|(_, v)| NSString::from_str(v))
            .collect();
        let fields = NSDictionary::from_slices(
            &keys.iter().map(|k| &**k).collect::<Vec<_>>(),
            &values.iter().map(|v| &**v).collect::<Vec<_>>(),
        );
        let Some(http) = NSHTTPURLResponse::initWithURL_statusCode_HTTPVersion_headerFields(
            NSHTTPURLResponse::alloc(),
            &url,
            response.status as isize,
            Some(&NSString::from_str("HTTP/1.1")),
            Some(&fields),
        ) else {
            return;
        };
        task.didReceiveResponse(&http);
        task.didReceiveData(&NSData::with_bytes(&response.body));
        task.didFinish();
    }
}

impl Observer {
    /// The download `download` ended: no longer observed, out of the list.
    fn saved(&self, download: &WKDownload) -> Option<Saving> {
        let mut downloads = self.ivars().downloads.borrow_mut();
        let at = downloads
            .iter()
            .position(|saving| std::ptr::eq(&*saving.download, download))?;
        let saving = downloads.remove(at);
        // SAFETY: the observation `destination` added, on the main thread.
        unsafe {
            saving
                .progress
                .removeObserver_forKeyPath(self, &NSString::from_str("completedUnitCount"))
        };
        Some(saving)
    }
    fn new(web: &WebView, mtm: MainThreadMarker) -> Retained<Self> {
        let object = mtm.alloc().set_ivars(ObserverState {
            web: web.downgrade(),
            view: RefCell::new(Weak::default()),
            new_windows: web.options().new_windows,
            downloads: RefCell::new(Vec::new()),
        });
        // SAFETY: NSObject's `init`, on a fresh allocation with its ivars set.
        unsafe { msg_send![super(object), init] }
    }
    fn post(&self, delivery: impl FnOnce(&WebView) + 'static) {
        let web = self.ivars().web.clone();
        zgx::native_view::post(move || {
            if let Some(web) = web.upgrade() {
                delivery(&web);
            }
        });
    }
    /// Report the page's navigation state as it is now.
    fn changed(&self) {
        self.changed_with(true);
    }
    /// The same, reading the back-forward list only if `history` (it walks
    /// up to a hundred items).
    fn changed_with(&self, history: bool) {
        let Some(view) = self.ivars().view.borrow().load() else {
            return;
        };
        let url = url_of(&view);
        // SAFETY: plain WKWebView getters on the main thread.
        let (title, loading, progress, back, forward) = unsafe {
            (
                view.title()
                    .map(|title| title.to_string())
                    .unwrap_or_default(),
                view.isLoading(),
                view.estimatedProgress(),
                view.canGoBack(),
                view.canGoForward(),
            )
        };
        let lists = history.then(|| self::history(&view));
        let full_screen = full_screen(&view);
        self.post(move |web| {
            let (old_url, old_title) = web
                .state()
                .with_untracked(|state| (state.url.clone(), state.title.clone()));
            web.update_state(|state| {
                if !url.is_empty() {
                    state.url = url.clone();
                }
                state.title = title.clone();
                state.loading = loading;
                state.progress = progress;
                state.can_go_back = back;
                state.can_go_forward = forward;
                state.full_screen = full_screen;
                if let Some((back_list, forward_list)) = lists {
                    state.back_list = back_list;
                    state.forward_list = forward_list;
                }
            });
            if !url.is_empty() && url != old_url {
                web.emit(WebViewEvent::UrlChanged { url });
            }
            if title != old_title {
                web.emit(WebViewEvent::TitleChanged { title });
            }
        });
    }
    fn failed(&self, view: &WKWebView, error: &NSError) {
        // NSURLErrorCancelled: a new navigation replaced this one.
        if error.code() == -999 {
            return;
        }
        // The URL that failed, not the page still showing (for a provisional
        // navigation they differ).
        let info = error.userInfo();
        let failing = |key: &str| info.objectForKey(&NSString::from_str(key));
        let url = failing("NSErrorFailingURLKey")
            .and_then(|url| url.downcast::<NSURL>().ok())
            .and_then(|url| url.absoluteString())
            .map(|url| url.to_string())
            .or_else(|| {
                failing("NSErrorFailingURLStringKey")
                    .and_then(|url| url.downcast::<NSString>().ok())
                    .map(|url| url.to_string())
            })
            .unwrap_or_else(|| url_of(view));
        let message = error.localizedDescription().to_string();
        self.post(move |web| {
            web.update_state(|state| {
                state.loading = false;
                state.error = Some(message.clone());
            });
            web.emit(WebViewEvent::NavigationFailed {
                url,
                error: message,
            });
        });
        self.changed();
    }
}

fn url_of(view: &WKWebView) -> String {
    // SAFETY: a plain getter on the main thread, nil-checked.
    unsafe { view.URL() }
        .and_then(|url| url.absoluteString())
        .map(|url| url.to_string())
        .unwrap_or_default()
}

/// A JavaScript value WebKit returned, as JSON.
fn json(value: Option<&AnyObject>) -> String {
    let Some(value) = value else {
        return "null".into();
    };
    if let Some(text) = value.downcast_ref::<NSString>() {
        return quote(&text.to_string());
    }
    if value.class().name().to_bytes() == b"__NSCFBoolean" {
        // SAFETY: an NSNumber (Foundation's boolean class), whose
        // `boolValue` returns BOOL.
        let on: bool = unsafe { msg_send![value, boolValue] };
        return if on { "true" } else { "false" }.into();
    }
    if let Some(number) = value.downcast_ref::<NSNumber>() {
        return number.stringValue().to_string();
    }
    if value.class().name().to_bytes() == b"NSNull" {
        return "null".into();
    }
    // SAFETY: NSJSONSerialization's class methods with their declared types
    // (`dataWithJSONObject:` only after `isValidJSONObject:` said yes, as it
    // requires; options an NSUInteger, a null error out-pointer). The NSData
    // it returns is autoreleased, alive for this call: its `bytes` and
    // `length` describe one buffer, read before returning. `description`
    // returns an autoreleased NSString, which `Retained` retains.
    unsafe {
        let class = objc2::class!(NSJSONSerialization);
        let valid: bool = msg_send![class, isValidJSONObject: value];
        if valid {
            let data: *mut AnyObject = msg_send![class, dataWithJSONObject: value, options: 0usize, error: std::ptr::null_mut::<*mut NSError>()];
            if let Some(data) = data.as_ref() {
                // `bytes` is `const void *`: objc2 checks the declared type.
                let bytes: *const std::ffi::c_void = msg_send![data, bytes];
                let length: usize = msg_send![data, length];
                if !bytes.is_null() {
                    return String::from_utf8_lossy(std::slice::from_raw_parts(
                        bytes.cast::<u8>(),
                        length,
                    ))
                    .into_owned();
                }
            }
        }
        let description: Retained<NSString> = msg_send![value, description];
        quote(&description.to_string())
    }
}
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

thread_local! {
    // Stores stay alive through process exit: releasing a data store in
    // AppKit's exit (Rust TLS destructors) removes its network session before
    // WebKit flushes cookies (learned in comet-native's browser).
    static STORES: ManuallyDrop<RefCell<HashMap<WebProfile, Retained<WKWebsiteDataStore>>>> =
        ManuallyDrop::new(RefCell::new(HashMap::new()));
}
fn data_store(profile: &WebProfile, mtm: MainThreadMarker) -> Retained<WKWebsiteDataStore> {
    // Each ephemeral view gets a store of its own.
    if *profile == WebProfile::Ephemeral {
        // SAFETY: a class factory with no precondition (main thread, by `mtm`).
        return unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) };
    }
    STORES.with(|stores| {
        stores
            .borrow_mut()
            .entry(profile.clone())
            // SAFETY: class factories on the main thread (`mtm`);
            // `dataStoreForIdentifier:` (macOS 14) only where it exists.
            .or_insert_with(|| unsafe {
                match profile {
                    WebProfile::Named(name) if named_stores_supported() => {
                        // From its text: `from_bytes` passes a byte array
                        // objc2's debug encoding check rejects (a crash).
                        let bytes = identifier(name);
                        let text: String = bytes
                            .iter()
                            .enumerate()
                            .map(|(i, b)| {
                                let dash = if matches!(i, 4 | 6 | 8 | 10) { "-" } else { "" };
                                format!("{dash}{b:02X}")
                            })
                            .collect();
                        let identifier = NSUUID::from_string(&NSString::from_str(&text))
                            .unwrap_or_else(|| NSUUID::from_bytes(bytes));
                        WKWebsiteDataStore::dataStoreForIdentifier(&identifier, mtm)
                    }
                    WebProfile::Named(_) => WKWebsiteDataStore::nonPersistentDataStore(mtm),
                    _ => WKWebsiteDataStore::defaultDataStore(mtm),
                }
            })
            .clone()
    })
}
/// Persistent stores of their own need macOS 14.
fn named_stores_supported() -> bool {
    // SAFETY: NSObject's `+respondsToSelector:`, returning BOOL.
    unsafe {
        msg_send![objc2::class!(WKWebsiteDataStore), respondsToSelector: sel!(dataStoreForIdentifier:)]
    }
}
/// A stable UUID for a profile name (FNV-1a, two lanes).
fn identifier(name: &str) -> [u8; 16] {
    let lane = |seed: u64| {
        name.bytes().fold(seed, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    };
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&lane(0xcbf2_9ce4_8422_2325).to_be_bytes());
    bytes[8..].copy_from_slice(&lane(0x6c62_272e_07bb_0142).to_be_bytes());
    // Version 5 shape, RFC 4122 variant.
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    bytes
}

/// A page: the `WKWebView` and its delegate.
pub(crate) struct MacWebView {
    view: Retained<WKWebView>,
    observer: Retained<Observer>,
    /// The page's custom-scheme servers, kept for as long as it is.
    _schemes: Vec<Retained<SchemeServer>>,
    messages: bool,
    transparent: bool,
    /// The last HTML shown and its base: WebKit reloads such a page by
    /// fetching its URL (`about:blank`, a blank page), so reload shows it again.
    html: RefCell<Option<Html>>,
    /// The app's zoom of the page (`WebView::set_zoom`) and the window's
    /// (`Ui::zoom`): the page shows at their product.
    zoom: Cell<(f64, f64)>,
}
/// A page's HTML and its base URL.
type Html = (Retained<NSString>, Option<Retained<NSURL>>);
impl MacWebView {
    pub fn new(web: &WebView, mtm: MainThreadMarker) -> Self {
        let options = web.options().clone();
        let observer = Observer::new(web, mtm);
        // WebKit throws (and the app aborts) on a scheme it serves itself or
        // one registered twice: leave those out.
        let mut taken = std::collections::HashSet::new();
        let handlers: Vec<&SchemeHandler> = options
            .schemes
            .iter()
            .filter(|handler| {
                let scheme = handler.scheme.as_str();
                // SAFETY: a class query with no precondition (main thread).
                let usable = zgx::web_view::is_app_scheme(scheme)
                    && !unsafe { WKWebView::handlesURLScheme(&NSString::from_str(scheme), mtm) }
                    && taken.insert(scheme.to_owned());
                if !usable {
                    eprintln!(
                        "zgx: web view scheme `{scheme}` left out: not a scheme an app can serve, or given twice"
                    );
                }
                usable
            })
            .collect();
        let schemes: Vec<Retained<SchemeServer>> = handlers
            .iter()
            .map(|handler| SchemeServer::new(web, handler, mtm))
            .collect();
        // SAFETY: configuring a fresh configuration on the main thread. Each
        // scheme is one WebKit does not serve, given once (WebKit raises
        // otherwise); the configuration retains its scheme handlers and the
        // content controller its message handler (removed in `Drop`).
        let view = unsafe {
            let configuration = WKWebViewConfiguration::new(mtm);
            // A page's `requestFullscreen`: WebKit's own full-screen window
            // (macOS 12.3; off before, as WKWebView's default).
            let preferences = configuration.preferences();
            let fullscreen: bool =
                msg_send![&*preferences, respondsToSelector: sel!(setElementFullscreenEnabled:)];
            if fullscreen {
                preferences.setElementFullscreenEnabled(true);
            }
            configuration.setWebsiteDataStore(&data_store(&options.profile, mtm));
            for (handler, server) in handlers.iter().zip(&schemes) {
                configuration.setURLSchemeHandler_forURLScheme(
                    Some(ProtocolObject::from_ref(&**server)),
                    &NSString::from_str(&handler.scheme),
                );
            }
            let content = configuration.userContentController();
            for script in &options.scripts {
                let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                    mtm.alloc(),
                    &NSString::from_str(&script.source),
                    WKUserScriptInjectionTime::AtDocumentStart,
                    script.main_frame_only,
                );
                content.addUserScript(&script);
            }
            {
                let content = configuration.userContentController();
                content.addScriptMessageHandler_name(
                    ProtocolObject::from_ref(&*observer),
                    &NSString::from_str(PRINT),
                );
                let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                    mtm.alloc(),
                    &NSString::from_str(PRINT_SCRIPT),
                    WKUserScriptInjectionTime::AtDocumentStart,
                    true,
                );
                content.addUserScript(&script);
            }
            if options.messages {
                let content = configuration.userContentController();
                content.addScriptMessageHandler_name(
                    ProtocolObject::from_ref(&*observer),
                    &NSString::from_str(CHANNEL),
                );
                let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                    mtm.alloc(),
                    &NSString::from_str(CHANNEL_SCRIPT),
                    WKUserScriptInjectionTime::AtDocumentStart,
                    true,
                );
                content.addUserScript(&script);
            }
            // SAFETY: WKWebView's designated initializer, on a fresh
            // allocation of zgx's subclass, which adds no state.
            let view: Retained<ZgxWebView> = msg_send![
                super(mtm.alloc::<ZgxWebView>().set_ivars(())),
                initWithFrame: NSRect::ZERO,
                configuration: &*configuration
            ];
            Retained::into_super(view)
        };
        *observer.ivars().view.borrow_mut() = Weak::from_retained(&view);
        // SAFETY: the delegates and KVO registrations hold `observer`
        // unretained: `Self` keeps it, and `Drop` unregisters it before it
        // goes (a null KVO context is allowed). `setInspectable:` (BOOL) is
        // sent only where it exists; `drawsBackground` is WKWebView's
        // key-value coded setting, and `setUnderPageBackgroundColor:` takes
        // an NSColor (macOS 12; older WebKit raises an exception, not UB).
        unsafe {
            view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*observer)));
            view.setUIDelegate(Some(ProtocolObject::from_ref(&*observer)));
            view.setAllowsBackForwardNavigationGestures(true);
            for key in OBSERVED.iter().chain(full_screen_key(&view)) {
                view.addObserver_forKeyPath_options_context(
                    &observer,
                    &NSString::from_str(key),
                    NSKeyValueObservingOptions::empty(),
                    std::ptr::null_mut(),
                );
            }
            if let Some(agent) = &options.user_agent {
                view.setCustomUserAgent(Some(&NSString::from_str(agent)));
            }
            let inspectable: bool = msg_send![&*view, respondsToSelector: sel!(setInspectable:)];
            if inspectable {
                let _: () = msg_send![&*view, setInspectable: options.inspectable];
            }
            if options.transparent {
                let no = NSNumber::new_bool(false);
                let _: () = msg_send![&*view, setValue: &*no, forKey: &*NSString::from_str("drawsBackground")];
                let clear = NSColor::clearColor();
                let _: () = msg_send![&*view, setUnderPageBackgroundColor: &*clear];
            }
        }
        Self {
            view,
            observer,
            _schemes: schemes,
            messages: options.messages,
            transparent: options.transparent,
            html: RefCell::new(None),
            zoom: Cell::new((1., 1.)),
        }
    }
    pub fn view(&self) -> &NSView {
        &self.view
    }
    /// Follow the window's zoom: the page lays out at it, as under a
    /// browser's zoom, times the app's own.
    pub(crate) fn set_window_zoom(&self, zoom: f64) {
        let (page, window) = self.zoom.get();
        if window != zoom {
            self.zoom.set((page, zoom));
            self.show_zoom();
        }
    }
    fn show_zoom(&self) {
        let (page, window) = self.zoom.get();
        // SAFETY: WKWebView's `setPageZoom:` (CGFloat), on the main thread
        // (`MacWebView` lives there), sent only where the view responds to it.
        unsafe {
            let has: bool = msg_send![&*self.view, respondsToSelector: sel!(setPageZoom:)];
            if has {
                let _: () = msg_send![&*self.view, setPageZoom: page * window];
            }
        }
    }
    pub fn transparent(&self) -> bool {
        self.transparent
    }
    /// The colour WebKit shows beyond the page (and under tiles not drawn
    /// yet, while it catches up with a new size).
    pub fn under_page_color(&self) -> Retained<NSColor> {
        // SAFETY: a null-resettable getter (never nil) on the main thread.
        unsafe { self.view.underPageBackgroundColor() }
    }
    fn has_focus(&self) -> bool {
        self.view
            .window()
            .and_then(|window| window.firstResponder())
            .and_then(|responder| responder.downcast::<NSView>().ok())
            .is_some_and(|view| view.isDescendantOf(&self.view))
    }
    fn load(&self, url: &str) {
        let parsed = NSURL::URLWithString(&NSString::from_str(url));
        let parsed = parsed.or_else(|| {
            // A path: show the file.
            std::path::Path::new(url)
                .is_absolute()
                .then(|| NSURL::fileURLWithPath(&NSString::from_str(url)))
        });
        let Some(parsed) = parsed else {
            let url = url.to_owned();
            self.observer.post(move |web| {
                web.update_state(|state| state.error = Some("Not a URL".into()));
                web.emit(WebViewEvent::NavigationFailed {
                    url,
                    error: "Not a URL".into(),
                });
            });
            return;
        };
        // SAFETY: WKWebView's loading methods on the main thread, with URLs
        // made just above.
        unsafe {
            if parsed.isFileURL() {
                let directory = parsed
                    .URLByDeletingLastPathComponent()
                    .unwrap_or(parsed.clone());
                self.view
                    .loadFileURL_allowingReadAccessToURL(&parsed, &directory);
            } else {
                self.view
                    .loadRequest(&NSURLRequest::requestWithURL(&parsed));
            }
        }
    }
}
impl WebViewBackend for MacWebView {
    fn run(&self, command: WebViewCommand) {
        // SAFETY: WKWebView's methods on the main thread (zgx runs commands
        // there). The script completion is a `void (^)(id, NSError *)`
        // block, as WebKit calls it, on the main thread. The unpublished
        // `_inspector` (an object, nil-checked, whose `show` takes nothing)
        // is sent only where the view responds to it.
        unsafe {
            match command {
                WebViewCommand::LoadUrl(url) => {
                    self.html.replace(None);
                    self.load(&url);
                }
                WebViewCommand::LoadHtml { html, base_url } => {
                    let base =
                        base_url.and_then(|base| NSURL::URLWithString(&NSString::from_str(&base)));
                    let html = NSString::from_str(&html);
                    self.view.loadHTMLString_baseURL(&html, base.as_deref());
                    self.html.replace(Some((html, base)));
                }
                WebViewCommand::Reload => {
                    // Still on the HTML's page (not a link followed from it).
                    let html = self.html.borrow().clone().filter(|(_, base)| {
                        let shown = base
                            .as_ref()
                            .and_then(|base| base.absoluteString())
                            .map_or_else(|| "about:blank".into(), |url| url.to_string());
                        url_of(&self.view) == shown
                    });
                    match html {
                        Some((html, base)) => {
                            self.view.loadHTMLString_baseURL(&html, base.as_deref());
                        }
                        None => {
                            self.view.reload();
                        }
                    }
                }
                WebViewCommand::Stop => self.view.stopLoading(),
                WebViewCommand::GoBack => {
                    self.view.goBack();
                }
                WebViewCommand::GoForward => {
                    self.view.goForward();
                }
                WebViewCommand::GoToHistory(n) => {
                    if let Some(item) = self.view.backForwardList().itemAtIndex(n as isize) {
                        self.view.goToBackForwardListItem(&item);
                    }
                }
                WebViewCommand::Evaluate { script, reply } => {
                    let reply = RefCell::new(reply);
                    let web = self.observer.ivars().web.clone();
                    let completion =
                        block2::RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
                            let Some(reply) = reply.borrow_mut().take() else {
                                return;
                            };
                            let result: ScriptResult = match error.as_ref() {
                                Some(error) => Err(error.localizedDescription().to_string()),
                                None => Ok(json(value.as_ref())),
                            };
                            let web = web.clone();
                            zgx::native_view::post(move || {
                                // The reply runs even if the controller went.
                                let _ = web.upgrade();
                                reply(result);
                            });
                        });
                    self.view.evaluateJavaScript_completionHandler(
                        &NSString::from_str(&script),
                        Some(&completion),
                    );
                }
                WebViewCommand::Focus(focus) => {
                    let Some(window) = self.view.window() else {
                        return;
                    };
                    if focus && !self.has_focus() {
                        window.makeFirstResponder(Some(&self.view));
                    } else if !focus && self.has_focus() {
                        window.makeFirstResponder(
                            window
                                .contentView()
                                .as_deref()
                                .map(|view| -> &objc2_app_kit::NSResponder { view }),
                        );
                    }
                }
                WebViewCommand::OpenDevtools => {
                    let has: bool = msg_send![&*self.view, respondsToSelector: sel!(_inspector)];
                    if has {
                        let inspector: *mut AnyObject = msg_send![&*self.view, _inspector];
                        if let Some(inspector) = inspector.as_ref() {
                            let _: () = msg_send![inspector, show];
                        }
                    }
                }
                WebViewCommand::SetZoom(zoom) => {
                    self.zoom.set((zoom, self.zoom.get().1));
                    self.show_zoom();
                }
            }
        }
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn print(&self) {
        print_page(&self.view);
    }
    fn find(
        &self,
        text: &str,
        options: &zgx::web_view::FindOptions,
        reply: Box<dyn FnOnce(zgx::web_view::FindResult)>,
    ) {
        let mtm = MainThreadMarker::from(&*self.view);
        let reply = Cell::new(Some(reply));
        // SAFETY: WKWebView's find (macOS 11) on the main thread, where
        // WebKit calls the completion, with the result it passes.
        unsafe {
            let config = objc2_web_kit::WKFindConfiguration::new(mtm);
            config.setBackwards(options.backwards);
            config.setCaseSensitive(options.case_sensitive);
            config.setWraps(true);
            let done = block2::RcBlock::new(
                move |result: std::ptr::NonNull<objc2_web_kit::WKFindResult>| {
                    let found = result.as_ref().matchFound();
                    if let Some(reply) = reply.take() {
                        let mut result = zgx::web_view::FindResult::default();
                        result.found = found;
                        reply(result);
                    }
                },
            );
            self.view.findString_withConfiguration_completionHandler(
                &NSString::from_str(text),
                Some(&config),
                &done,
            );
        }
    }
    fn clear_find(&self) {
        // WebKit selects a match as it finds it, and has no call to stop.
        self.run(WebViewCommand::Evaluate {
            script: "window.getSelection().removeAllRanges()".into(),
            reply: None,
        });
    }
}
impl Drop for MacWebView {
    fn drop(&mut self) {
        // SAFETY: undoes `new`'s registrations (each key observed there,
        // the delegates, the message handler) while `observer` is still
        // alive, so WebKit keeps no unretained pointer to it; main thread.
        unsafe {
            if self.has_focus()
                && let Some(window) = self.view.window()
            {
                window.makeFirstResponder(
                    window
                        .contentView()
                        .as_deref()
                        .map(|view| -> &objc2_app_kit::NSResponder { view }),
                );
            }
            for key in OBSERVED.iter().chain(full_screen_key(&self.view)) {
                self.view
                    .removeObserver_forKeyPath(&self.observer, &NSString::from_str(key));
            }
            self.view.setNavigationDelegate(None);
            self.view.setUIDelegate(None);
            let content = self.view.configuration().userContentController();
            content.removeScriptMessageHandlerForName(&NSString::from_str(PRINT));
            if self.messages {
                content.removeScriptMessageHandlerForName(&NSString::from_str(CHANNEL));
            }
            self.view.stopLoading();
        }
        self.view.removeFromSuperview();
    }
}

/// The platform page behind `web`, made now if it has none.
pub(crate) fn backend(web: &WebView, mtm: MainThreadMarker) -> Rc<dyn WebViewBackend> {
    if let Some(backend) = web.backend()
        && backend.as_any().is::<MacWebView>()
    {
        return backend;
    }
    let backend: Rc<dyn WebViewBackend> = Rc::new(MacWebView::new(web, mtm));
    web.attach(backend.clone());
    backend
}

#[cfg(test)]
mod tests {
    use super::json;
    use objc2::{msg_send, rc::Retained, runtime::AnyObject};
    use objc2_foundation::{NSData, NSError};

    /// What a page posts as an object (`postMessage({ … })`): a dictionary,
    /// back as its JSON. A debug build checks each message's declared types.
    #[test]
    fn a_posted_object_arrives_as_its_json() {
        let text = br#"{"key":"u","meta":true,"list":[1,"two",null]}"#;
        let data = NSData::with_bytes(text);
        // SAFETY: NSJSONSerialization's class method with its declared types
        // (NSData, NSUInteger options, a null error out-pointer); it returns
        // an autoreleased object or nil, retained here.
        let object: Option<Retained<AnyObject>> = unsafe {
            msg_send![
                objc2::class!(NSJSONSerialization),
                JSONObjectWithData: &*data,
                options: 0usize,
                error: std::ptr::null_mut::<*mut NSError>()
            ]
        };
        let object = object.expect("valid JSON");
        let back: serde_json::Value = serde_json::from_str(&json(Some(&object))).unwrap();
        let expected: serde_json::Value = serde_json::from_slice(text).unwrap();
        assert_eq!(back, expected);
    }
}
