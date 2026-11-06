//! Files and photos a person chooses on iOS: `UIDocumentPickerViewController`,
//! and `PHPickerViewController` where only images are asked for. What is chosen
//! comes as files of the app's own, which it may read as it likes.
use crate::file_dialog::{FileDialogOptions, Kind};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol},
};
use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

#[link(name = "PhotosUI", kind = "framework")]
#[link(name = "UniformTypeIdentifiers", kind = "framework")]
unsafe extern "C" {}

#[derive(Default)]
struct Shared {
    done: Option<Vec<PathBuf>>,
    waker: Option<Waker>,
    /// Photos still being copied, and those that were.
    left: usize,
    copied: Vec<PathBuf>,
}
impl Shared {
    fn finish(&mut self, paths: Vec<PathBuf>) {
        self.done = Some(paths);
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}
struct Wait(Arc<Mutex<Shared>>);
impl Future for Wait {
    type Output = Vec<PathBuf>;
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut shared = self.0.lock().expect("no panic holds it");
        match shared.done.take() {
            Some(paths) => Poll::Ready(paths),
            None => {
                shared.waker = Some(context.waker().clone());
                Poll::Pending
            }
        }
    }
}

fn path_of(url: &AnyObject) -> Option<PathBuf> {
    // SAFETY: NSURL's and NSString's getters; the bytes are read before they go.
    unsafe {
        let path: Option<Retained<AnyObject>> = msg_send![url, path];
        let path = path?;
        let bytes: *const std::ffi::c_char = msg_send![&*path, UTF8String];
        (!bytes.is_null()).then(|| {
            PathBuf::from(
                std::ffi::CStr::from_ptr(bytes)
                    .to_string_lossy()
                    .into_owned(),
            )
        })
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Arc<Mutex<Shared>>]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    impl Delegate {
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn picked(&self, _picker: &AnyObject, urls: &AnyObject) {
            let mut paths = Vec::new();
            // SAFETY: NSArray's getters, on the main thread UIKit calls on.
            unsafe {
                let count: usize = msg_send![urls, count];
                for index in 0..count {
                    let url: Retained<AnyObject> = msg_send![urls, objectAtIndex: index];
                    // A folder is the person's: it stays open for the app's life.
                    let _: bool = msg_send![&*url, startAccessingSecurityScopedResource];
                    paths.extend(path_of(&url));
                }
            }
            self.finish(paths);
        }
        #[unsafe(method(documentPickerWasCancelled:))]
        fn cancelled(&self, _picker: &AnyObject) {
            self.finish(Vec::new());
        }
        #[unsafe(method(picker:didFinishPicking:))]
        fn photos(&self, picker: &AnyObject, results: &AnyObject) {
            // SAFETY: UIKit's and Foundation's methods, on the main thread; each
            // handler keeps what it needs, and runs once.
            unsafe {
                let _: () = msg_send![picker, dismissViewControllerAnimated: true, completion: std::ptr::null::<AnyObject>()];
                let count: usize = msg_send![results, count];
                if count == 0 {
                    self.finish(Vec::new());
                    return;
                }
                self.ivars().lock().expect("no panic holds it").left = count;
                let image = crate::ios_system::ns_string("public.image");
                for index in 0..count {
                    let result: Retained<AnyObject> = msg_send![results, objectAtIndex: index];
                    let provider: Retained<AnyObject> = msg_send![&*result, itemProvider];
                    let shared = self.ivars().clone();
                    let handler = block2::RcBlock::new(
                        move |url: *mut AnyObject, _error: *mut AnyObject| {
                            copy_photo(url.as_ref(), &shared);
                        },
                    );
                    let _: Option<Retained<AnyObject>> = msg_send![&*provider,
                        loadFileRepresentationForTypeIdentifier: &*image,
                        completionHandler: &*handler];
                }
            }
        }
    }
);
impl Delegate {
    fn finish(&self, paths: Vec<PathBuf>) {
        self.ivars()
            .lock()
            .expect("no panic holds it")
            .finish(paths);
    }
}

/// The photo at `url` goes when its handler returns: kept, in the app's cache.
fn copy_photo(url: Option<&AnyObject>, shared: &Mutex<Shared>) {
    let copied = url.and_then(path_of).and_then(|from| {
        let name = from.file_name()?.to_owned();
        let to = std::env::temp_dir().join(name);
        std::fs::copy(&from, &to).ok()?;
        Some(to)
    });
    let mut shared = shared.lock().expect("no panic holds it");
    shared.copied.extend(copied);
    shared.left -= 1;
    if shared.left == 0 {
        let paths = std::mem::take(&mut shared.copied);
        shared.finish(paths);
    }
}

const IMAGES: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "heic", "heif", "webp", "tiff", "bmp",
];

/// Whether what is asked for is only images: the photo library's to show.
fn only_images(options: &FileDialogOptions) -> bool {
    !options.filters.is_empty()
        && options.filters.iter().all(|filter| {
            filter
                .extensions
                .iter()
                .all(|extension| IMAGES.contains(&extension.to_ascii_lowercase().as_str()))
        })
}

/// What `kind` chooses from, as `UTType`s.
fn content_types(kind: &Kind, options: &FileDialogOptions) -> Retained<AnyObject> {
    // SAFETY: Foundation's and UniformTypeIdentifiers' constructors.
    unsafe {
        let types: Retained<AnyObject> = msg_send![class!(NSMutableArray), array];
        let by_id = |id: &str| -> Option<Retained<AnyObject>> {
            let id = crate::ios_system::ns_string(id);
            msg_send![class!(UTType), typeWithIdentifier: &*id]
        };
        if matches!(kind, Kind::Folder | Kind::Folders) {
            if let Some(folder) = by_id("public.folder") {
                let _: () = msg_send![&*types, addObject: &*folder];
            }
            return types;
        }
        for extension in options.filters.iter().flat_map(|filter| &filter.extensions) {
            let extension = crate::ios_system::ns_string(extension);
            let found: Option<Retained<AnyObject>> =
                msg_send![class!(UTType), typeWithFilenameExtension: &*extension];
            if let Some(found) = found {
                let _: () = msg_send![&*types, addObject: &*found];
            }
        }
        let count: usize = msg_send![&*types, count];
        if count == 0
            && let Some(any) = by_id("public.item")
        {
            let _: () = msg_send![&*types, addObject: &*any];
        }
        types
    }
}

/// Ask the person; none chosen is no paths.
pub(crate) async fn select(kind: Kind, options: FileDialogOptions) -> Option<Vec<PathBuf>> {
    // Save: a file of the app's own, which the Files app shows.
    if matches!(kind, Kind::Save) {
        let home = std::env::var_os("HOME")?;
        let name = options.file_name.unwrap_or_else(|| "Untitled".into());
        return Some(vec![PathBuf::from(home).join("Documents").join(name)]);
    }
    let mtm = MainThreadMarker::new()?;
    let controller = crate::ios_system::root_controller()?;
    let shared = Arc::new(Mutex::new(Shared::default()));
    let delegate = Delegate::alloc(mtm).set_ivars(shared.clone());
    let many = matches!(kind, Kind::OpenFiles | Kind::Folders);
    // SAFETY: UIKit's and PhotosUI's constructors and setters, on the main thread.
    let delegate = unsafe {
        let delegate: Retained<Delegate> = msg_send![super(delegate), init];
        let picker: Retained<AnyObject> = if only_images(&options) {
            let config: Retained<AnyObject> = msg_send![class!(PHPickerConfiguration), new];
            let _: () = msg_send![&*config, setSelectionLimit: if many { 0isize } else { 1isize }];
            let filter: Retained<AnyObject> = msg_send![class!(PHPickerFilter), imagesFilter];
            let _: () = msg_send![&*config, setFilter: &*filter];
            let picker: Retained<AnyObject> = msg_send![msg_send![class!(PHPickerViewController), alloc], initWithConfiguration: &*config];
            let _: () = msg_send![&*picker, setDelegate: &*delegate];
            picker
        } else {
            let types = content_types(&kind, &options);
            // A folder is opened where it lies, a file copied in for the app.
            let copy = !matches!(kind, Kind::Folder | Kind::Folders);
            let picker: Retained<AnyObject> = msg_send![msg_send![class!(UIDocumentPickerViewController), alloc],
                initForOpeningContentTypes: &*types,
                asCopy: copy];
            let _: () = msg_send![&*picker, setAllowsMultipleSelection: many];
            let _: () = msg_send![&*picker, setDelegate: &*delegate];
            picker
        };
        let _: () = msg_send![&*controller,
            presentViewController: &*picker,
            animated: true,
            completion: std::ptr::null::<AnyObject>()];
        delegate
    };
    let paths = Wait(shared).await;
    drop(delegate);
    Some(paths)
}
