//! iOS's clipboard, `UIPasteboard`, in the shape of the arboard calls
//! `clipboard.rs` makes. arboard has no iOS backend of its own. Reading it may
//! show iOS's paste prompt, so nothing here reads until the person pastes.
//! Images aren't carried yet.
use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use std::{borrow::Cow, path::PathBuf};

/// What went wrong.
#[derive(Debug)]
pub(crate) struct Error(&'static str);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}

fn general() -> Retained<AnyObject> {
    // SAFETY: UIPasteboard's class method.
    unsafe { msg_send![class!(UIPasteboard), generalPasteboard] }
}
fn nsstring(text: &str) -> Retained<AnyObject> {
    let text = std::ffi::CString::new(text.replace('\0', "")).unwrap_or_default();
    // SAFETY: NSString's constructor, over a NUL-terminated string.
    unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
}

pub(crate) struct ImageData<'a> {
    pub width: usize,
    pub height: usize,
    pub bytes: Cow<'a, [u8]>,
}

pub(crate) struct Clipboard;
impl Clipboard {
    pub(crate) fn new() -> Result<Self, Error> {
        Ok(Self)
    }
    pub(crate) fn get_text(&mut self) -> Result<String, Error> {
        // SAFETY: UIPasteboard's getter, and an NSString's UTF-8 view copied at once.
        unsafe {
            let text: Option<Retained<AnyObject>> = msg_send![&*general(), string];
            let text = text.ok_or(Error("no text on the pasteboard"))?;
            let bytes: *const std::ffi::c_char = msg_send![&*text, UTF8String];
            Ok(std::ffi::CStr::from_ptr(bytes)
                .to_string_lossy()
                .into_owned())
        }
    }
    pub(crate) fn set_text(&mut self, text: String) -> Result<(), Error> {
        // SAFETY: UIPasteboard's setter.
        unsafe {
            let _: () = msg_send![&*general(), setString: &*nsstring(&text)];
        }
        Ok(())
    }
    /// Text and its rich form, as two representations of one item.
    pub(crate) fn set_html(&mut self, html: String, text: Option<String>) -> Result<(), Error> {
        // SAFETY: Foundation's constructors, and UIPasteboard's setter with an
        // item of UTI to value, as it documents.
        unsafe {
            let mut keys = vec![nsstring("public.html")];
            let mut values = vec![nsstring(&html)];
            if let Some(text) = text {
                keys.push(nsstring("public.utf8-plain-text"));
                values.push(nsstring(&text));
            }
            let key_refs: Vec<*const AnyObject> = keys.iter().map(Retained::as_ptr).collect();
            let value_refs: Vec<*const AnyObject> = values.iter().map(Retained::as_ptr).collect();
            let item: Retained<AnyObject> = msg_send![class!(NSDictionary),
                dictionaryWithObjects: value_refs.as_ptr(),
                forKeys: key_refs.as_ptr(),
                count: key_refs.len()];
            let item_ref = Retained::as_ptr(&item);
            let items: Retained<AnyObject> = msg_send![class!(NSArray), arrayWithObject: item_ref];
            let _: () = msg_send![&*general(), setItems: &*items];
        }
        Ok(())
    }
    pub(crate) fn get(&mut self) -> Get {
        Get
    }
    pub(crate) fn get_image(&mut self) -> Result<ImageData<'static>, Error> {
        Err(Error("images on iOS's pasteboard aren't carried yet"))
    }
    pub(crate) fn set_image(&mut self, _: ImageData<'_>) -> Result<(), Error> {
        Err(Error("images on iOS's pasteboard aren't carried yet"))
    }
}

/// What `Clipboard::get` reads through.
pub(crate) struct Get;
impl Get {
    pub(crate) fn file_list(self) -> Result<Vec<PathBuf>, Error> {
        Ok(Vec::new())
    }
}
