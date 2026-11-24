//! Windows' Text Services Framework for zgx's editors: a thread manager, a
//! document manager per window, and a context over an `ITextStoreACP2` of
//! zgx's own (`TextStore`, `text_store.rs`), so input methods and text
//! services read the focused editor's text around the caret, place their
//! candidates by the clause they convert, and insert where it is: the emoji
//! panel, dictation and the touch keyboard type into it, and an input
//! method's composition is its preedit.
//!
//! The default; IMM32's path (winit's) instead with
//! `Application::text_services(false)`, or `ZGX_TSF=0` (which, like
//! `ZGX_TSF=1`, overrides what the app chose). It is IMM32's too where the
//! framework does not start.
//!
//! Locks: Windows asks for one (`RequestLock`) to read or edit, and hears of
//! edits it didn't make only when no lock is held. A request while another
//! lock is held is granted after it (`after_dispatch`), as the asynchronous
//! grant `TS_S_ASYNC` says; a synchronous one then is refused
//! (`TS_E_SYNCHRONOUS`), as the interface allows. zgx calls into the
//! framework only after its dispatch, so a lock never meets a borrowed
//! editor.
use crate::text_store::{Change, TextStore};
use std::{cell::RefCell, rc::Rc};
use windows::{
    Win32::{
        Foundation::{E_INVALIDARG, E_NOTIMPL, E_UNEXPECTED, HWND, POINT, RECT},
        Graphics::Gdi::ClientToScreen,
        System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemAlloc, FORMATETC, IDataObject},
            Variant::{VARIANT, VT_I4, VT_UNKNOWN},
        },
        UI::{
            TextServices::{
                CLSID_TF_CategoryMgr, CLSID_TF_DisplayAttributeMgr, CLSID_TF_ThreadMgr,
                GUID_PROP_ATTRIBUTE, GUID_PROP_INPUTSCOPE, ITextStoreACP2, ITextStoreACP2_Impl,
                ITextStoreACPSink, ITfCategoryMgr, ITfCompositionView, ITfContext,
                ITfContextOwnerCompositionServices, ITfContextOwnerCompositionSink,
                ITfContextOwnerCompositionSink_Impl, ITfDisplayAttributeInfo,
                ITfDisplayAttributeMgr, ITfDocumentMgr, ITfEditRecord, ITfInputScope,
                ITfInputScope_Impl, ITfRange, ITfRangeACP, ITfSource, ITfTextEditSink,
                ITfTextEditSink_Impl, ITfThreadMgr, InputScope, TEXT_STORE_LOCK_FLAGS,
                TEXT_STORE_TEXT_CHANGE_FLAGS, TF_ATTR_CONVERTED, TF_ATTR_FIXEDCONVERTED,
                TF_ATTR_TARGET_CONVERTED, TF_ATTR_TARGET_NOTCONVERTED, TF_DISPLAYATTRIBUTE,
                TF_IAS_QUERYONLY, TS_AE_END, TS_AE_START, TS_AS_ATTR_CHANGE, TS_AS_LAYOUT_CHANGE,
                TS_AS_SEL_CHANGE, TS_AS_TEXT_CHANGE, TS_ATTRVAL, TS_DEFAULT_SELECTION,
                TS_E_INVALIDPOS, TS_E_NOLOCK, TS_E_SYNCHRONOUS, TS_LC_CHANGE, TS_LF_READ,
                TS_LF_READWRITE, TS_LF_SYNC, TS_RT_PLAIN, TS_RUNINFO, TS_S_ASYNC, TS_SELECTION_ACP,
                TS_SELECTIONSTYLE, TS_SS_NOHIDDENTEXT, TS_STATUS, TS_TEXTCHANGE,
            },
            WindowsAndMessaging::GetClientRect,
        },
    },
    core::{BOOL, BSTR, GUID, HRESULT, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result, implement},
};
use zgx::{scene::Rect, text_edit::ClauseStyle, widgets::EditorHandle};

/// The view cookie: an editor shows its text in one.
const VIEW: u32 = 1;

/// What the COM object and the host share.
struct Shared {
    store: TextStore,
    sink: Option<ITextStoreACPSink>,
    /// AdviseSink notification mask (`TS_AS_*`).
    sink_mask: u32,
    /// The lock held now (`TS_LF_READ` or `TS_LF_READWRITE` bits), if any.
    lock: u32,
    /// The editor dropped a composition, told with its text, which is still
    /// to be ended (`Tsf::after_dispatch`).
    cancelled: bool,
    /// A lock asked for while one couldn't be granted, granted after.
    pending: Option<u32>,
    hwnd: HWND,
    scale: f64,
    /// The document's length when the input method last heard of it.
    told_len: usize,
    layout_changed: bool,
    /// What reads an input method's display attributes (how it marks its
    /// composition's clauses), made when one first composes.
    attributes: Option<(ITfCategoryMgr, ITfDisplayAttributeMgr)>,
    /// The framework asked for the input scope (`RequestSupportedAttrs`),
    /// to fetch with `RetrieveRequestedAttrs`.
    scope_asked: bool,
    /// The input scope it was last told of (`retell_scope`).
    told_scope: i32,
}

/// The Text Services Framework for one window.
pub(crate) struct Tsf {
    thread: ITfThreadMgr,
    document: ITfDocumentMgr,
    _context: ITfContext,
    shared: Rc<RefCell<Shared>>,
    focused: bool,
    /// The context's text-edit sink (`OnEndEdit`), advised while it lives.
    edits: Option<u32>,
}

/// Whether windows use the framework (`Application::text_services`).
pub(crate) static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Whether a window uses the framework.
fn enabled() -> bool {
    choose(
        zgx::env::var_os("ZGX_TSF").as_deref(),
        ENABLED.load(std::sync::atomic::Ordering::Relaxed),
    )
}
/// `ZGX_TSF=0` or `1` if set (the user's), else what the app chose.
fn choose(env: Option<&std::ffi::OsStr>, app: bool) -> bool {
    match env {
        Some(value) => value != "0",
        None => app,
    }
}

impl Tsf {
    /// The framework for window `hwnd` (`scale`: its physical pixels per
    /// point), when it is on and starts; `None` keeps IMM32's path.
    pub(crate) fn new(hwnd: HWND, scale: f64) -> Option<Self> {
        if !enabled() {
            return None;
        }
        // SAFETY: COM was initialized on this thread by winit (OLE, for drag
        // and drop); every pointer passed is valid for the call.
        unsafe {
            let thread: ITfThreadMgr =
                CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER).ok()?;
            let client = thread.Activate().ok()?;
            let document = thread.CreateDocumentMgr().ok()?;
            let shared = Rc::new(RefCell::new(Shared {
                store: TextStore::new(),
                sink: None,
                sink_mask: 0,
                cancelled: false,
                lock: 0,
                pending: None,
                hwnd,
                scale,
                told_len: 0,
                layout_changed: false,
                attributes: None,
                scope_asked: false,
                told_scope: 0,
            }));
            let store: IUnknown = Store(shared.clone()).into();
            let (mut context, mut cookie) = (None, 0);
            document
                .CreateContext(client, 0, &store, &mut context, &mut cookie)
                .ok()?;
            let context = context?;
            document.Push(&context).ok()?;
            // Told after each of an input method's edits, to read how it
            // marked its composition then.
            let edits = context
                .cast::<ITfSource>()
                .and_then(|source| source.AdviseSink(&ITfTextEditSink::IID, &store))
                .ok();
            Some(Self {
                thread,
                document,
                _context: context,
                shared,
                focused: false,
                edits,
            })
        }
    }
    /// The framework for winit's `window` (as `new`).
    pub(crate) fn for_window(window: &winit::window::Window, scale: f64) -> Option<Self> {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
            return None;
        };
        Self::new(HWND(handle.hwnd.get() as *mut _), scale)
    }
    /// The editor taking the input method now, or none.
    pub(crate) fn focus(&mut self, editor: Option<EditorHandle>) {
        let on = editor.is_some();
        // A composition open in the editor losing focus ends there: the
        // input method stops composing (its OnEndComposition commits it
        // into that editor), and the store commits it if it didn't.
        // (No borrow is held across the call: it calls back into the store.)
        let composing = self.shared.borrow().store.is_composing();
        if composing
            && let Ok(services) = self._context.cast::<ITfContextOwnerCompositionServices>()
        {
            // SAFETY: the context's own services; no lock is held.
            let _ = unsafe { services.TerminateComposition(None) };
        }
        {
            let mut shared = self.shared.borrow_mut();
            shared.store.focus(editor);
            shared.told_len = shared.store.len();
        }
        retell_scope(&self.shared);
        if on != self.focused {
            let hwnd = self.shared.borrow().hwnd;
            // SAFETY: the document manager outlives the association.
            let _ = unsafe {
                if on {
                    self.thread.AssociateFocus(hwnd, &self.document)
                } else {
                    self.thread.AssociateFocus(hwnd, None)
                }
            };
            self.focused = on;
        }
    }
    pub(crate) fn set_scale(&self, scale: f64) {
        self.shared.borrow_mut().scale = scale;
    }
    /// Where the input method's candidates go moved.
    pub(crate) fn layout_changed(&self) {
        self.shared.borrow_mut().layout_changed = true;
    }
    /// After zgx's dispatch: tell the input method what changed that it
    /// didn't do, then grant a lock it asked for meanwhile.
    pub(crate) fn after_dispatch(&self) {
        if self.shared.borrow().lock != 0 {
            return;
        }
        // The focused editor's input mode set while it has focus.
        retell_scope(&self.shared);
        tell(&self.shared);
        let (sink, mask, layout, cancelled) = {
            let mut shared = self.shared.borrow_mut();
            // (Told, it may hold a lock now.)
            if shared.lock != 0 {
                return;
            }
            (
                shared.sink.clone(),
                shared.sink_mask,
                std::mem::take(&mut shared.layout_changed),
                std::mem::take(&mut shared.cancelled),
            )
        };
        if layout
            && mask & TS_AS_LAYOUT_CHANGE != 0
            && let Some(sink) = &sink
        {
            // SAFETY: the sink is alive while advised.
            let _ = unsafe { sink.OnLayoutChange(TS_LC_CHANGE, VIEW) };
        }
        // The editor dropped the composition (the app set its text or its
        // selection): the input method ends its own, with nothing of it left
        // to commit (its OnEndComposition finds none). No borrow is held
        // across the call: it calls back into the store.
        if cancelled
            && let Ok(services) = self._context.cast::<ITfContextOwnerCompositionServices>()
        {
            // SAFETY: the context's own services; no lock is held.
            let _ = unsafe { services.TerminateComposition(None) };
        }
        let pending = self.shared.borrow_mut().pending.take();
        if let Some(flags) = pending {
            let _ = grant(&self.shared, flags);
        }
    }
}

impl Drop for Tsf {
    fn drop(&mut self) {
        // SAFETY: deactivating the thread manager this activated.
        unsafe {
            if let (Some(cookie), Ok(source)) = (self.edits, self._context.cast::<ITfSource>()) {
                let _ = source.UnadviseSink(cookie);
            }
            let _ = self.document.Pop(0);
            let _ = self.thread.Deactivate();
        }
    }
}

fn acp(at: usize) -> i32 {
    i32::try_from(at).unwrap_or(i32::MAX)
}
fn at(acp: i32) -> usize {
    usize::try_from(acp).unwrap_or(0)
}
/// The `len` units at `text`: none at a null pointer, as an empty text (a
/// deletion's) may come.
///
/// # Safety
/// `text`, unless null, points at `len` units that outlive the result.
unsafe fn units(text: &PCWSTR, len: u32) -> &[u16] {
    match text.is_null() {
        true => &[],
        // SAFETY: as the caller promises.
        false => unsafe { std::slice::from_raw_parts(text.0, len as usize) },
    }
}

/// A COM method's `body`: a panic in it fails the call (`E_UNEXPECTED`)
/// instead of ending the app. Windows calls the store from inside its own
/// message handling (`PeekMessage`), and a panic can't unwind out through
/// a COM call: Rust aborts the process there, with no window to say so.
fn guard<T>(body: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(body))
        .unwrap_or_else(|_| Err(E_UNEXPECTED.into()))
}

/// Tell the input method the input scope changed: focus moved to an editor
/// of another mode, or the focused one's mode changed. One context serves
/// every editor in the window, and an input method reads the scope again
/// only when told. (Nothing is borrowed across the call: the framework asks
/// for a lock in it.)
fn retell_scope(shared: &Rc<RefCell<Shared>>) {
    let (sink, mask, len) = {
        let mut s = shared.borrow_mut();
        let scope = s.store.input_scope();
        if scope == s.told_scope {
            return;
        }
        s.told_scope = scope;
        (s.sink.clone(), s.sink_mask, s.store.len())
    };
    if let Some(sink) = sink
        && mask & TS_AS_ATTR_CHANGE != 0
    {
        // SAFETY: the sink is alive while advised; no lock is held.
        let _ = unsafe { sink.OnAttrsChange(0, acp(len), &[GUID_PROP_INPUTSCOPE]) };
    }
}

/// Tell the input method what changed that it didn't do. No lock is held,
/// and nothing is borrowed across the calls: the framework asks for a lock
/// in them.
fn tell(shared: &Rc<RefCell<Shared>>) {
    let (sink, mask, changes) = {
        let mut shared = shared.borrow_mut();
        let told = shared.told_len;
        let changes = shared.store.changes(told);
        // (Most dispatches: nothing.)
        if changes.is_empty() {
            return;
        }
        shared.told_len = shared.store.len();
        shared.cancelled |= changes.contains(&Change::CompositionCancelled);
        (shared.sink.clone(), shared.sink_mask, changes)
    };
    let Some(sink) = sink else { return };
    for change in changes {
        // SAFETY: the sink is alive while advised; the change is valid.
        let _ = unsafe {
            match change {
                Change::Text {
                    start,
                    old_end,
                    new_end,
                } if mask & TS_AS_TEXT_CHANGE != 0 => sink.OnTextChange(
                    TEXT_STORE_TEXT_CHANGE_FLAGS(0),
                    &TS_TEXTCHANGE {
                        acpStart: acp(start),
                        acpOldEnd: acp(old_end),
                        acpNewEnd: acp(new_end),
                    },
                ),
                Change::Selection if mask & TS_AS_SEL_CHANGE != 0 => sink.OnSelectionChange(),
                _ => Ok(()),
            }
        };
    }
}

/// A lock the store has granted: given back as this drops, a panic under
/// it included (`guard` fails the call then), so the next request isn't
/// told the store is busy for good.
struct Held<'a>(&'a RefCell<Shared>);
impl<'a> Held<'a> {
    fn take(shared: &'a RefCell<Shared>, flags: u32) -> Self {
        shared.borrow_mut().lock = flags & TS_LF_READWRITE.0;
        Self(shared)
    }
}
impl Drop for Held<'_> {
    fn drop(&mut self) {
        if let Ok(mut shared) = self.0.try_borrow_mut() {
            shared.lock = 0;
        }
    }
}

/// Grant lock `flags` now: hold it while the sink reads or edits. (An edit of
/// the app's not told by now isn't told here: told as a lock is asked for, the
/// framework ends the composition begun under it and what was typed is lost;
/// told after, it moves that composition's range. The app's turns end telling
/// them: `Tsf::after_dispatch`.)
fn grant(shared: &Rc<RefCell<Shared>>, flags: u32) -> HRESULT {
    let held = Held::take(shared, flags);
    let sink = shared.borrow().sink.clone();
    // SAFETY: the sink is alive while advised.
    let result = sink.map_or(HRESULT(0), |sink| unsafe {
        sink.OnLockGranted(TEXT_STORE_LOCK_FLAGS(flags & TS_LF_READWRITE.0))
            .map_or_else(|e| e.code(), |()| HRESULT(0))
    });
    // What the input method edited under the lock it knows of: the
    // document's length from here. (Only read, with an edit of the app's
    // still to tell, it knows what it did.)
    {
        let mut s = shared.borrow_mut();
        if s.store.told() {
            s.told_len = s.store.len();
        }
    }
    drop(held);
    result
}

#[implement(ITextStoreACP2, ITfContextOwnerCompositionSink, ITfTextEditSink)]
struct Store(Rc<RefCell<Shared>>);

impl Store {
    fn read(&self) -> Result<std::cell::RefMut<'_, Shared>> {
        let shared = self.0.borrow_mut();
        if shared.lock & TS_LF_READ.0 == 0 {
            return Err(TS_E_NOLOCK.into());
        }
        Ok(shared)
    }
    fn write(&self) -> Result<std::cell::RefMut<'_, Shared>> {
        let shared = self.0.borrow_mut();
        if shared.lock & TS_LF_READWRITE.0 != TS_LF_READWRITE.0 {
            return Err(TS_E_NOLOCK.into());
        }
        Ok(shared)
    }
    /// A rect in the window's points as the screen's pixels.
    fn screen(shared: &Shared, r: Rect) -> RECT {
        let s = shared.scale;
        let mut origin = POINT::default();
        // SAFETY: a valid window and point.
        unsafe {
            let _ = ClientToScreen(shared.hwnd, &mut origin);
        }
        RECT {
            left: origin.x + (f64::from(r.x) * s) as i32,
            top: origin.y + (f64::from(r.y) * s) as i32,
            right: origin.x + (f64::from(r.x + r.width.max(1.)) * s) as i32,
            bottom: origin.y + (f64::from(r.y + r.height) * s) as i32,
        }
    }
}

impl ITextStoreACP2_Impl for Store_Impl {
    fn AdviseSink(&self, riid: *const GUID, punk: Ref<IUnknown>, mask: u32) -> Result<()> {
        // SAFETY: `riid` is a valid GUID from the caller.
        if unsafe { *riid } != ITextStoreACPSink::IID {
            return Err(E_INVALIDARG.into());
        }
        let sink: ITextStoreACPSink = punk.ok()?.cast()?;
        let mut shared = self.0.borrow_mut();
        shared.sink = Some(sink);
        shared.sink_mask = mask;
        Ok(())
    }
    fn UnadviseSink(&self, _punk: Ref<IUnknown>) -> Result<()> {
        let mut shared = self.0.borrow_mut();
        shared.sink = None;
        shared.sink_mask = 0;
        Ok(())
    }
    fn RequestLock(&self, flags: u32) -> Result<HRESULT> {
        guard(|| {
            let busy = {
                let shared = self.0.borrow();
                shared.lock != 0
            };
            if busy {
                if flags & TS_LF_SYNC != 0 {
                    return Ok(TS_E_SYNCHRONOUS);
                }
                let mut shared = self.0.borrow_mut();
                // A write asked for wins over a read.
                let wanted = shared.pending.unwrap_or(0) | (flags & TS_LF_READWRITE.0);
                shared.pending = Some(wanted);
                return Ok(TS_S_ASYNC);
            }
            Ok(grant(&self.0, flags))
        })
    }
    fn GetStatus(&self) -> Result<TS_STATUS> {
        Ok(TS_STATUS {
            dwDynamicFlags: 0,
            dwStaticFlags: TS_SS_NOHIDDENTEXT,
        })
    }
    fn QueryInsert(
        &self,
        start: i32,
        end: i32,
        _cch: u32,
        result_start: *mut i32,
        result_end: *mut i32,
    ) -> Result<()> {
        guard(|| {
            let len = self.0.borrow().store.len();
            if start < 0 || start > end || at(end) > len {
                return Err(E_INVALIDARG.into());
            }
            // SAFETY: the caller's out pointers, each checked for null.
            unsafe {
                if !result_start.is_null() {
                    *result_start = start;
                }
                if !result_end.is_null() {
                    *result_end = end;
                }
            }
            Ok(())
        })
    }
    fn GetSelection(
        &self,
        index: u32,
        count: u32,
        selection: *mut TS_SELECTION_ACP,
        fetched: *mut u32,
    ) -> Result<()> {
        guard(|| {
            let shared = self.read()?;
            if count == 0 || (index != 0 && index != TS_DEFAULT_SELECTION) {
                // SAFETY: the caller's out pointer.
                unsafe { *fetched = 0 };
                return Ok(());
            }
            let (start, end, at_start) = shared.store.selection();
            // SAFETY: the caller's out pointers, for `count` (at least one).
            unsafe {
                *selection = TS_SELECTION_ACP {
                    acpStart: acp(start),
                    acpEnd: acp(end),
                    style: TS_SELECTIONSTYLE {
                        ase: if at_start { TS_AE_START } else { TS_AE_END },
                        fInterimChar: false.into(),
                    },
                };
                *fetched = 1;
            }
            Ok(())
        })
    }
    fn SetSelection(&self, count: u32, selection: *const TS_SELECTION_ACP) -> Result<()> {
        guard(|| {
            let mut shared = self.write()?;
            if count == 0 {
                return Ok(());
            }
            // SAFETY: at least one selection from the caller.
            let s = unsafe { *selection };
            shared
                .store
                .set_selection(at(s.acpStart), at(s.acpEnd), s.style.ase == TS_AE_START);
            Ok(())
        })
    }
    fn GetText(
        &self,
        start: i32,
        end: i32,
        text: PWSTR,
        text_wanted: u32,
        text_given: *mut u32,
        runs: *mut TS_RUNINFO,
        runs_wanted: u32,
        runs_given: *mut u32,
        next: *mut i32,
    ) -> Result<()> {
        guard(|| {
            let shared = self.read()?;
            let len = shared.store.len();
            if start < 0 || at(start) > len || (end >= 0 && (end < start || at(end) > len)) {
                return Err(TS_E_INVALIDPOS.into());
            }
            let start_u = at(start);
            let end_u = if end < 0 { len } else { at(end) };
            let units = shared.store.text(start_u, end_u);
            let n = units.len().min(text_wanted as usize);
            let run_count = if text_wanted > 0 { n } else { units.len() };
            let advance = if text_wanted > 0 {
                n
            } else if runs_wanted > 0 {
                run_count
            } else {
                0
            };
            // SAFETY: the caller's buffers, `text_wanted` and `runs_wanted` long.
            unsafe {
                if !text.is_null() && n > 0 {
                    std::ptr::copy_nonoverlapping(units.as_ptr(), text.0, n);
                }
                if !text_given.is_null() {
                    *text_given = n as u32;
                }
                if runs_wanted > 0 && !runs.is_null() && run_count > 0 {
                    *runs = TS_RUNINFO {
                        uCount: run_count as u32,
                        r#type: TS_RT_PLAIN,
                    };
                    if !runs_given.is_null() {
                        *runs_given = 1;
                    }
                } else if !runs_given.is_null() {
                    *runs_given = 0;
                }
                if !next.is_null() {
                    *next = acp(start_u + advance);
                }
            }
            Ok(())
        })
    }
    fn SetText(
        &self,
        _flags: u32,
        start: i32,
        end: i32,
        text: &PCWSTR,
        cch: u32,
    ) -> Result<TS_TEXTCHANGE> {
        guard(|| {
            let mut shared = self.write()?;
            // SAFETY: `cch` units at `text`, from the caller.
            let units = unsafe { units(text, cch) };
            let range = shared
                .store
                .replace(at(start), at(end), units)
                .unwrap_or(at(start)..at(start));
            shared.told_len = shared.store.len();
            Ok(TS_TEXTCHANGE {
                acpStart: start,
                acpOldEnd: end,
                acpNewEnd: acp(range.end),
            })
        })
    }
    fn GetFormattedText(&self, _start: i32, _end: i32) -> Result<IDataObject> {
        Err(E_NOTIMPL.into())
    }
    fn GetEmbedded(&self, _at: i32, _service: *const GUID, _riid: *const GUID) -> Result<IUnknown> {
        Err(E_NOTIMPL.into())
    }
    fn QueryInsertEmbedded(
        &self,
        _service: *const GUID,
        _format: *const FORMATETC,
    ) -> Result<BOOL> {
        Ok(false.into())
    }
    fn InsertEmbedded(
        &self,
        _flags: u32,
        _start: i32,
        _end: i32,
        _data: Ref<IDataObject>,
    ) -> Result<TS_TEXTCHANGE> {
        Err(E_NOTIMPL.into())
    }
    fn InsertTextAtSelection(
        &self,
        flags: u32,
        text: &PCWSTR,
        cch: u32,
        start_out: *mut i32,
        end_out: *mut i32,
        change: *mut TS_TEXTCHANGE,
    ) -> Result<()> {
        guard(|| {
            let query_only = flags & TF_IAS_QUERYONLY.0 != 0;
            let mut shared = if query_only {
                self.read()?
            } else {
                self.write()?
            };
            let (start, end, _) = shared.store.selection();
            let range = if query_only {
                start..end
            } else {
                // SAFETY: `cch` units at `text`, from the caller.
                let units = unsafe { units(text, cch) };
                let range = shared
                    .store
                    .replace(start, end, units)
                    .unwrap_or(start..start);
                shared.told_len = shared.store.len();
                range
            };
            // SAFETY: the caller's out pointers, each optional.
            unsafe {
                if !start_out.is_null() {
                    *start_out = acp(range.start);
                }
                if !end_out.is_null() {
                    *end_out = acp(range.end);
                }
                if !change.is_null() {
                    *change = TS_TEXTCHANGE {
                        acpStart: acp(start),
                        acpOldEnd: acp(end),
                        acpNewEnd: acp(range.end),
                    };
                }
            }
            Ok(())
        })
    }
    fn InsertEmbeddedAtSelection(
        &self,
        _flags: u32,
        _data: Ref<IDataObject>,
        _start: *mut i32,
        _end: *mut i32,
        _change: *mut TS_TEXTCHANGE,
    ) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn RequestSupportedAttrs(&self, _flags: u32, count: u32, attrs: *const GUID) -> Result<()> {
        // The one attribute the store has: the editor's input scope.
        self.0.borrow_mut().scope_asked = asks_scope(count, attrs);
        Ok(())
    }
    fn RequestAttrsAtPosition(
        &self,
        _at: i32,
        count: u32,
        attrs: *const GUID,
        _flags: u32,
    ) -> Result<()> {
        // The scope is the whole document's, at every position.
        self.0.borrow_mut().scope_asked = asks_scope(count, attrs);
        Ok(())
    }
    fn RequestAttrsTransitioningAtPosition(
        &self,
        _at: i32,
        _count: u32,
        _attrs: *const GUID,
        _flags: u32,
    ) -> Result<()> {
        Ok(())
    }
    fn FindNextAttrTransition(
        &self,
        _start: i32,
        halt: i32,
        _count: u32,
        _attrs: *const GUID,
        _flags: u32,
        next: *mut i32,
        found: *mut BOOL,
        offset: *mut i32,
    ) -> Result<()> {
        // SAFETY: the caller's out pointers.
        unsafe {
            *next = halt;
            *found = false.into();
            *offset = 0;
        }
        Ok(())
    }
    fn RetrieveRequestedAttrs(
        &self,
        count: u32,
        values: *mut TS_ATTRVAL,
        fetched: *mut u32,
    ) -> Result<()> {
        guard(|| {
            let scope = {
                let mut shared = self.0.borrow_mut();
                std::mem::take(&mut shared.scope_asked).then(|| shared.store.input_scope())
            };
            let (Some(scope), true) = (scope, count > 0 && !values.is_null()) else {
                // SAFETY: the caller's out pointer.
                unsafe { *fetched = 0 };
                return Ok(());
            };
            // The scope as the framework takes it: an ITfInputScope in a
            // VT_UNKNOWN, which the caller releases.
            let mut value = VARIANT::default();
            let scopes: ITfInputScope = Scopes(InputScope(scope)).into();
            // SAFETY: a zeroed VARIANT, its type and pointer set as one; the
            // caller's buffer, `count` long, and out pointer.
            unsafe {
                let inner = &mut *value.Anonymous.Anonymous;
                inner.vt = VT_UNKNOWN;
                inner.Anonymous.punkVal = std::mem::ManuallyDrop::new(Some(scopes.into()));
                values.write(TS_ATTRVAL {
                    idAttr: GUID_PROP_INPUTSCOPE,
                    dwOverlapId: 0,
                    varValue: value,
                });
                *fetched = 1;
            }
            Ok(())
        })
    }
    fn GetEndACP(&self) -> Result<i32> {
        guard(|| Ok(acp(self.read()?.store.len())))
    }
    fn GetActiveView(&self) -> Result<u32> {
        Ok(VIEW)
    }
    fn GetACPFromPoint(&self, _view: u32, _point: *const POINT, _flags: u32) -> Result<i32> {
        Err(E_NOTIMPL.into())
    }
    fn GetTextExt(
        &self,
        _view: u32,
        start: i32,
        end: i32,
        rect: *mut RECT,
        clipped: *mut BOOL,
    ) -> Result<()> {
        guard(|| {
            let shared = self.read()?;
            // Where the range shows; the caret's place where its text has no
            // lines to ask (a password's).
            let end = if end < 0 { shared.store.len() } else { at(end) };
            let area = shared
                .store
                .area(at(start), end)
                .or_else(|| shared.store.editor().map(EditorHandle::input_method_area))
                .unwrap_or_default();
            // SAFETY: the caller's out pointers.
            unsafe {
                *rect = Store::screen(&shared, area);
                *clipped = false.into();
            }
            Ok(())
        })
    }
    fn GetScreenExt(&self, _view: u32) -> Result<RECT> {
        let shared = self.0.borrow();
        let mut client = RECT::default();
        // SAFETY: a valid window and rect.
        unsafe {
            let _ = GetClientRect(shared.hwnd, &mut client);
            let mut origin = POINT::default();
            let _ = ClientToScreen(shared.hwnd, &mut origin);
            client.left += origin.x;
            client.right += origin.x;
            client.top += origin.y;
            client.bottom += origin.y;
        }
        Ok(client)
    }
}

impl ITfContextOwnerCompositionSink_Impl for Store_Impl {
    fn OnStartComposition(&self, view: Ref<ITfCompositionView>) -> Result<BOOL> {
        guard(|| {
            // Over the range it opens on: empty to compose at the caret, over
            // committed text to reconvert it.
            let (start, len) = extent(view.ok()?).unwrap_or((0, 0));
            self.0
                .borrow_mut()
                .store
                .start_composition(start..start + len);
            Ok(true.into())
        })
    }
    fn OnUpdateComposition(
        &self,
        _view: Ref<ITfCompositionView>,
        _range: Ref<ITfRange>,
    ) -> Result<()> {
        // The composition's text came through SetText; its clauses (display
        // attributes) are read once the input method's edit ends (`OnEndEdit`).
        Ok(())
    }
    fn OnEndComposition(&self, _view: Ref<ITfCompositionView>) -> Result<()> {
        guard(|| {
            let mut shared = self.0.borrow_mut();
            shared.store.end_composition();
            shared.told_len = shared.store.len();
            Ok(())
        })
    }
}

impl ITfTextEditSink_Impl for Store_Impl {
    fn OnEndEdit(
        &self,
        context: Ref<ITfContext>,
        read: u32,
        _record: Ref<ITfEditRecord>,
    ) -> Result<()> {
        guard(|| {
            // (No borrow is held across the framework's calls.)
            let attributes = {
                let mut shared = self.0.borrow_mut();
                if !shared.store.is_composing() {
                    return Ok(());
                }
                if shared.attributes.is_none() {
                    // SAFETY: COM is initialized on this thread (`Tsf::new`).
                    shared.attributes = unsafe {
                        CoCreateInstance::<_, ITfCategoryMgr>(
                            &CLSID_TF_CategoryMgr,
                            None,
                            CLSCTX_INPROC_SERVER,
                        )
                        .and_then(|category| {
                            CoCreateInstance::<_, ITfDisplayAttributeMgr>(
                                &CLSID_TF_DisplayAttributeMgr,
                                None,
                                CLSCTX_INPROC_SERVER,
                            )
                            .map(|display| (category, display))
                        })
                        .ok()
                    };
                }
                shared.attributes.clone()
            };
            let (Some(context), Some((category, display))) = (context.ok().ok(), attributes) else {
                return Ok(());
            };
            let clauses = clauses(context, read, &category, &display).unwrap_or_default();
            self.0.borrow_mut().store.mark_clauses(&clauses);
            Ok(())
        })
    }
}

/// How the input method marks the document's text (its display attributes'
/// ranges), read with edit cookie `read`: each range and the clause it is.
fn clauses(
    context: &ITfContext,
    read: u32,
    category: &ITfCategoryMgr,
    display: &ITfDisplayAttributeMgr,
) -> Result<Vec<(std::ops::Range<usize>, ClauseStyle)>> {
    let mut clauses = Vec::new();
    // SAFETY: the context's own property and ranges, read within the edit
    // session `read` is for; every out pointer is valid for its call.
    unsafe {
        let property = context.GetProperty(&GUID_PROP_ATTRIBUTE)?;
        let mut ranges = None;
        property.EnumRanges(read, &mut ranges, None::<&ITfRange>)?;
        let Some(ranges) = ranges else {
            return Ok(clauses);
        };
        loop {
            let (mut range, mut fetched) = ([None], 0);
            if ranges.Next(&mut range, &mut fetched).is_err() || fetched == 0 {
                break;
            }
            let Some(range) = range[0].take() else {
                break;
            };
            let value = property.GetValue(read, &range)?;
            let value = &value.Anonymous.Anonymous;
            if value.vt != VT_I4 {
                continue;
            }
            let Ok(guid) = category.GetGUID(value.Anonymous.lVal as u32) else {
                continue;
            };
            let mut info: Option<ITfDisplayAttributeInfo> = None;
            if display
                .GetDisplayAttributeInfo(&guid, &mut info, std::ptr::null_mut())
                .is_err()
            {
                continue;
            }
            let mut attribute = TF_DISPLAYATTRIBUTE::default();
            let Some(info) = info else { continue };
            if info.GetAttributeInfo(&mut attribute).is_err() {
                continue;
            }
            let style = match attribute.bAttr {
                TF_ATTR_TARGET_CONVERTED | TF_ATTR_TARGET_NOTCONVERTED => ClauseStyle::Target,
                TF_ATTR_CONVERTED | TF_ATTR_FIXEDCONVERTED => ClauseStyle::Converted,
                // Input, an input error, another mark: typed, not converted.
                _ => ClauseStyle::Input,
            };
            let range: ITfRangeACP = range.cast()?;
            let (mut start, mut len) = (0, 0);
            range.GetExtent(&mut start, &mut len)?;
            clauses.push((at(start)..at(start) + at(len), style));
        }
    }
    Ok(clauses)
}

/// Whether `count` attributes at `attrs` ask for the input scope.
fn asks_scope(count: u32, attrs: *const GUID) -> bool {
    if attrs.is_null() {
        return false;
    }
    // SAFETY: the caller's `count` attributes.
    unsafe { std::slice::from_raw_parts(attrs, count as usize) }.contains(&GUID_PROP_INPUTSCOPE)
}

/// An editor's input scope (`text_store::input_scope`), as the framework
/// reads it: one scope, no phrases or grammar.
#[implement(ITfInputScope)]
struct Scopes(InputScope);

impl ITfInputScope_Impl for Scopes_Impl {
    fn GetInputScopes(&self, scopes: *mut *mut InputScope, count: *mut u32) -> Result<()> {
        // SAFETY: memory the caller frees with CoTaskMemFree, one scope
        // long; the caller's out pointers.
        unsafe {
            let array = CoTaskMemAlloc(std::mem::size_of::<InputScope>()) as *mut InputScope;
            if array.is_null() {
                return Err(windows::Win32::Foundation::E_OUTOFMEMORY.into());
            }
            array.write(self.0);
            *scopes = array;
            *count = 1;
        }
        Ok(())
    }
    fn GetPhrase(&self, phrases: *mut *mut BSTR, count: *mut u32) -> Result<()> {
        // SAFETY: the caller's out pointers.
        unsafe {
            *phrases = std::ptr::null_mut();
            *count = 0;
        }
        Err(E_NOTIMPL.into())
    }
    fn GetRegularExpression(&self) -> Result<BSTR> {
        Err(E_NOTIMPL.into())
    }
    fn GetSRGS(&self) -> Result<BSTR> {
        Err(E_NOTIMPL.into())
    }
    fn GetXML(&self) -> Result<BSTR> {
        Err(E_NOTIMPL.into())
    }
}

/// A composition's range in the document: its start and length.
fn extent(view: &ITfCompositionView) -> Option<(usize, usize)> {
    // SAFETY: a range the composition gives, read through its ACP view.
    unsafe {
        let range: ITfRangeACP = view.GetRange().ok()?.cast().ok()?;
        let (mut start, mut len) = (0, 0);
        range.GetExtent(&mut start, &mut len).ok()?;
        Some((at(start), at(len)))
    }
}

#[cfg(test)]
mod tests {
    //! The store as the framework drives it, through its COM interfaces,
    //! with no framework: what zgx hands Windows. (What the touch keyboard
    //! does with it is Windows'.)
    use super::*;
    use windows::Win32::{
        Foundation::E_FAIL,
        UI::TextServices::{
            ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContextComposition,
            ITfEditSession, ITfEditSession_Impl, TF_DEFAULT_SELECTION, TF_ES_READWRITE, TF_ES_SYNC,
            TF_SELECTION,
        },
    };
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::TextServices::{
            IS_DIGITS, IS_EMAIL_SMTPEMAILADDRESS, ITextStoreACPSink_Impl, TsLayoutCode,
        },
    };
    use zgx::{compose::prelude::*, text_edit::InputMode, widgets::Ui};

    /// The framework is the default; the app turns it off, and the user's
    /// `ZGX_TSF` overrides the app either way.
    #[test]
    fn the_framework_is_the_default_and_the_environment_overrides_the_app() {
        use std::ffi::OsStr;
        assert!(choose(None, true), "the default");
        assert!(!choose(None, false), "the app's IMM32");
        assert!(!choose(Some(OsStr::new("0")), true), "the user's IMM32");
        assert!(choose(Some(OsStr::new("1")), false), "the user's framework");
    }

    /// A sink that keeps the attributes it was told changed.
    #[implement(ITextStoreACPSink)]
    struct Told(Rc<RefCell<Vec<GUID>>>);
    impl ITextStoreACPSink_Impl for Told_Impl {
        fn OnTextChange(
            &self,
            _flags: TEXT_STORE_TEXT_CHANGE_FLAGS,
            _change: *const TS_TEXTCHANGE,
        ) -> Result<()> {
            Ok(())
        }
        fn OnSelectionChange(&self) -> Result<()> {
            Ok(())
        }
        fn OnLayoutChange(&self, _code: TsLayoutCode, _view: u32) -> Result<()> {
            Ok(())
        }
        fn OnStatusChange(&self, _flags: u32) -> Result<()> {
            Ok(())
        }
        fn OnAttrsChange(
            &self,
            _start: i32,
            _end: i32,
            count: u32,
            attrs: *const GUID,
        ) -> Result<()> {
            // SAFETY: the caller's `count` attributes.
            let attrs = unsafe { std::slice::from_raw_parts(attrs, count as usize) };
            self.0.borrow_mut().extend_from_slice(attrs);
            Ok(())
        }
        fn OnLockGranted(&self, _flags: TEXT_STORE_LOCK_FLAGS) -> Result<()> {
            Ok(())
        }
        fn OnStartEditTransaction(&self) -> Result<()> {
            Ok(())
        }
        fn OnEndEditTransaction(&self) -> Result<()> {
            Ok(())
        }
    }

    /// The scopes the store answers for: GUID_PROP_INPUTSCOPE asked for,
    /// then fetched, its ITfInputScope read, as the framework does.
    fn scopes(store: &ITextStoreACP2) -> Vec<i32> {
        // SAFETY: the store's own methods, with buffers of the sizes given;
        // the scope array freed with CoTaskMemFree, the value released.
        unsafe {
            store
                .RequestSupportedAttrs(0, &[GUID_PROP_INPUTSCOPE])
                .unwrap();
            let mut values = [std::mem::zeroed::<TS_ATTRVAL>()];
            let mut fetched = 0;
            store
                .RetrieveRequestedAttrs(&mut values, &mut fetched)
                .unwrap();
            assert_eq!(fetched, 1);
            assert_eq!(values[0].idAttr, GUID_PROP_INPUTSCOPE);
            let inner = &values[0].varValue.Anonymous.Anonymous;
            assert_eq!(inner.vt, VT_UNKNOWN);
            // A reference of its own: the VARIANT releases the one it holds
            // as it drops, as the framework's VariantClear does.
            let unknown = (*inner.Anonymous.punkVal).clone().unwrap();
            let scope: ITfInputScope = unknown.cast().unwrap();
            let (mut array, mut count) = (std::ptr::null_mut(), 0);
            scope.GetInputScopes(&mut array, &mut count).unwrap();
            let read = std::slice::from_raw_parts(array, count as usize)
                .iter()
                .map(|s| s.0)
                .collect();
            CoTaskMemFree(Some(array as *const _));
            read
        }
    }

    /// Each editor's input mode is the scope the framework reads, told
    /// again when focus moves to another mode's editor and when the
    /// focused editor's mode changes.
    #[test]
    fn the_framework_reads_each_editors_input_scope() {
        let mut ui = Ui::new(400., 200.);
        let value = ui.signal(String::new());
        let view = ui.mount(
            column()
                .child(
                    text_input("Email", value.clone())
                        .id("email")
                        .input_mode(InputMode::Email),
                )
                .child(
                    text_input("PIN", value.clone())
                        .id("pin")
                        .input_mode(InputMode::Numeric),
                )
                .child(text_input("Name", value).id("name")),
        );
        ui.prepare_frame();
        let told = Rc::new(RefCell::new(Vec::new()));
        let sink: ITextStoreACPSink = Told(told.clone()).into();
        let shared = Rc::new(RefCell::new(Shared {
            store: TextStore::new(),
            sink: Some(sink),
            sink_mask: TS_AS_ATTR_CHANGE,
            cancelled: false,
            lock: 0,
            pending: None,
            hwnd: HWND::default(),
            scale: 1.,
            told_len: 0,
            layout_changed: false,
            attributes: None,
            scope_asked: false,
            told_scope: 0,
        }));
        let store: ITextStoreACP2 = Store(shared.clone()).into();
        let focus = |id: &str| {
            ui.input.focus(&ui.scene, view.find(id));
            shared.borrow_mut().store.focus(ui.focused_editor());
            retell_scope(&shared);
        };
        focus("email");
        assert_eq!(scopes(&store), [IS_EMAIL_SMTPEMAILADDRESS.0]);
        assert_eq!(told.borrow().as_slice(), [GUID_PROP_INPUTSCOPE]);
        focus("pin");
        assert_eq!(scopes(&store), [IS_DIGITS.0]);
        assert_eq!(told.borrow().len(), 2, "told as focus moved");
        // Its mode changed while it has focus: told after the dispatch.
        ui.focused_editor()
            .unwrap()
            .set_input_mode(InputMode::Email);
        retell_scope(&shared);
        assert_eq!(scopes(&store), [IS_EMAIL_SMTPEMAILADDRESS.0]);
        assert_eq!(told.borrow().len(), 3);
        // Nothing changed: nothing told.
        retell_scope(&shared);
        assert_eq!(told.borrow().len(), 3);
        focus("name");
        assert_eq!(scopes(&store), [0], "IS_DEFAULT");
        // Asked for nothing, the store fetches nothing.
        // SAFETY: the store's own method, with a buffer of the size given.
        unsafe {
            store.RequestSupportedAttrs(0, &[]).unwrap();
            let mut values = [std::mem::zeroed::<TS_ATTRVAL>()];
            let mut fetched = 7;
            store
                .RetrieveRequestedAttrs(&mut values, &mut fetched)
                .unwrap();
            assert_eq!(fetched, 0);
        }
    }

    /// Windows asks where text is from inside its own message handling, so
    /// it can ask after the focused editor left the scene and before zgx
    /// tells it focus moved: the store still answers (the caret's place,
    /// empty), where a panic in the COM call would end the app.
    #[test]
    fn the_store_answers_for_an_editor_gone_from_the_scene() {
        let mut ui = Ui::new(400., 200.);
        let value = ui.signal(String::from("hello"));
        let view = ui.mount(column().child(text_input("Name", value).id("name")));
        ui.prepare_frame();
        let shared = Rc::new(RefCell::new(Shared {
            store: TextStore::new(),
            sink: None,
            sink_mask: 0,
            cancelled: false,
            lock: 0,
            pending: None,
            hwnd: HWND::default(),
            scale: 1.,
            told_len: 0,
            layout_changed: false,
            attributes: None,
            scope_asked: false,
            told_scope: 0,
        }));
        let store: ITextStoreACP2 = Store(shared.clone()).into();
        ui.input.focus(&ui.scene, view.find("name"));
        shared.borrow_mut().store.focus(ui.focused_editor());
        ui.remove(view.node());
        ui.prepare_frame();
        // Under a read lock, as the framework asks.
        shared.borrow_mut().lock = TS_LF_READ.0;
        // SAFETY: the store's own methods, with out pointers valid for each.
        unsafe {
            let (mut rect, mut clipped) = (RECT::default(), BOOL::default());
            for (start, end) in [(0, 5), (2, 2), (0, -1)] {
                store
                    .GetTextExt(VIEW, start, end, &mut rect, &mut clipped)
                    .unwrap();
            }
            assert_eq!(store.GetEndACP().unwrap(), 5, "its text is the model's");
        }
    }

    /// A panic in a COM method fails the call instead of unwinding into
    /// Windows, which would abort the app.
    #[test]
    fn a_panic_in_a_com_method_fails_the_call() {
        let failed = guard::<()>(|| panic!("a bug"));
        assert_eq!(failed.unwrap_err().code(), E_UNEXPECTED);
        assert_eq!(guard(|| Ok(7)).unwrap(), 7);
    }

    /// A panic under a granted lock gives the lock back, so the store isn't
    /// busy for good after a call `guard` failed.
    #[test]
    fn a_panic_under_a_lock_gives_it_back() {
        let shared = RefCell::new(Shared {
            store: TextStore::new(),
            sink: None,
            sink_mask: 0,
            cancelled: false,
            lock: 0,
            pending: None,
            hwnd: HWND::default(),
            scale: 1.,
            told_len: 0,
            layout_changed: false,
            attributes: None,
            scope_asked: false,
            told_scope: 0,
        });
        let failed = guard::<()>(|| {
            let _held = Held::take(&shared, TS_LF_READWRITE.0);
            assert_ne!(shared.borrow().lock, 0, "held");
            panic!("a bug under the lock");
        });
        assert!(failed.is_err());
        assert_eq!(shared.borrow().lock, 0, "given back");
    }

    /// A text service, in the test's own process: the framework's thread
    /// manager activated for it, composing into zgx's own context as an
    /// input method composes (edit sessions; a composition started at the
    /// selection, its text set, then ended).
    struct Service {
        client: u32,
        context: ITfContext,
        composition: Rc<RefCell<Option<ITfComposition>>>,
    }

    /// An edit, run with its session's cookie.
    type Edit = Box<dyn FnOnce(u32) -> Result<()>>;
    /// One edit session.
    #[implement(ITfEditSession)]
    struct Session(RefCell<Option<Edit>>);
    impl ITfEditSession_Impl for Session_Impl {
        fn DoEditSession(&self, cookie: u32) -> Result<()> {
            self.0
                .borrow_mut()
                .take()
                .map_or(Ok(()), |edit| edit(cookie))
        }
    }

    /// Told when zgx ends the service's composition itself.
    #[implement(ITfCompositionSink)]
    struct Ended(Rc<RefCell<Option<ITfComposition>>>);
    impl ITfCompositionSink_Impl for Ended_Impl {
        fn OnCompositionTerminated(&self, _cookie: u32, _view: Ref<ITfComposition>) -> Result<()> {
            self.0.borrow_mut().take();
            Ok(())
        }
    }

    impl Service {
        fn new(tsf: &Tsf) -> Self {
            // SAFETY: the thread manager zgx activated on this thread.
            let client = unsafe { tsf.thread.Activate() }.unwrap();
            Self {
                client,
                context: tsf._context.clone(),
                composition: Rc::default(),
            }
        }
        /// `edit` in a synchronous read-write session, as zgx grants it.
        fn session(&self, edit: impl FnOnce(u32) -> Result<()> + 'static) {
            let session: ITfEditSession = Session(RefCell::new(Some(Box::new(edit)))).into();
            // SAFETY: zgx's context and the session, both alive; a
            // synchronous session runs before the call returns.
            unsafe {
                self.context
                    .RequestEditSession(self.client, &session, TF_ES_SYNC | TF_ES_READWRITE)
                    .unwrap()
                    .ok()
                    .unwrap();
            }
        }
        /// Compose `text` at the selection (a composition started there),
        /// or in place of what the open composition holds.
        fn compose(&self, text: &str) {
            let (context, composition) = (self.context.clone(), self.composition.clone());
            let text: Vec<u16> = text.encode_utf16().collect();
            self.session(move |cookie| {
                // SAFETY: within the session, with its cookie.
                unsafe {
                    let open = composition.borrow().clone();
                    let range = match &open {
                        Some(open) => open.GetRange()?,
                        None => {
                            let mut selection = [TF_SELECTION::default()];
                            let mut fetched = 0;
                            context.GetSelection(
                                cookie,
                                TF_DEFAULT_SELECTION,
                                &mut selection,
                                &mut fetched,
                            )?;
                            let [TF_SELECTION { range, .. }] = selection;
                            std::mem::ManuallyDrop::into_inner(range).ok_or(E_FAIL)?
                        }
                    };
                    if open.is_none() {
                        // (A null sink is refused: E_INVALIDARG.)
                        let ended: ITfCompositionSink = Ended(composition.clone()).into();
                        let started = context
                            .cast::<ITfContextComposition>()?
                            .StartComposition(cookie, &range, &ended)?;
                        *composition.borrow_mut() = Some(started);
                    }
                    range.SetText(cookie, 0, &text)
                }
            });
        }
        /// End the composition with `text` in its place: committed, or
        /// (empty) cancelled.
        fn end(&self, text: &str) {
            let composition = self.composition.borrow_mut().take().expect("composing");
            let text: Vec<u16> = text.encode_utf16().collect();
            self.session(move |cookie| {
                // SAFETY: within the session, with its cookie.
                unsafe {
                    composition.GetRange()?.SetText(cookie, 0, &text)?;
                    composition.EndComposition(cookie)
                }
            });
        }
    }

    /// An input method composing through the framework itself into zgx's
    /// context, in a hidden window of the test's (no key, no focus, no
    /// input method installed): beside an atom it commits without
    /// splitting it, before it too, a cancelled composition commits nothing,
    /// and one over a selection into the atom takes it whole. Each step is
    /// zgx's store's: the composition open only through its
    /// OnStartComposition, the text changed only under a lock it granted.
    #[test]
    fn an_input_method_composes_through_the_framework_beside_an_atom() {
        use windows::Win32::{
            System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx},
            UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
            },
        };
        if !interactive() {
            return;
        }
        // SAFETY: COM on this test's thread, as winit initializes it on the
        // UI thread (the framework wants a single-threaded apartment).
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        // SAFETY: a plain window of a system class, never shown.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!(""),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                200,
                100,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        let mut ui = Ui::new(400., 200.);
        let value = ui.signal("see ".to_owned());
        let view = ui.mount(text_area("Notes", value.clone()).size(300., 100.));
        ui.prepare_frame();
        ui.input.focus(&ui.scene, Some(view.node()));
        let editor = ui.focused_editor().unwrap();
        editor.set_selection(4, 4);
        let id = editor.insert_atom("@file.pdf", None, None, None).unwrap();
        let atoms = || editor.atoms();
        let Some(mut tsf) = Tsf::new(hwnd, 1.) else {
            // SAFETY: the window made above.
            let _ = unsafe { DestroyWindow(hwnd) };
            panic!("the framework didn't start");
        };
        tsf.focus(ui.focused_editor());
        let service = Service::new(&tsf);
        let composing = || tsf.shared.borrow().store.is_composing();
        let preedit = || editor.editor.borrow().preedit().map(|p| p.text.clone());

        // Right after the atom.
        editor.set_selection(13, 13);
        tsf.after_dispatch();
        service.compose("にほん");
        assert!(composing(), "zgx's store heard the composition start");
        assert_eq!(preedit().as_deref(), Some("にほん"));
        assert_eq!(value.get(), "see @file.pdf", "composing, nothing committed");
        service.end("日本");
        tsf.after_dispatch();
        assert!(!composing(), "and end");
        assert_eq!(preedit(), None);
        assert_eq!(value.get(), "see @file.pdf日本");
        assert_eq!(atoms(), [zgx::text_edit::Atom::new(id, 4..13)]);

        // Right before it.
        editor.set_selection(4, 4);
        tsf.after_dispatch();
        service.compose("ひ");
        service.end("火");
        tsf.after_dispatch();
        assert_eq!(value.get(), "see 火@file.pdf日本");
        assert_eq!(atoms(), [zgx::text_edit::Atom::new(id, 7..16)]);

        // Cancelled: nothing of it committed.
        service.compose("x");
        assert!(composing());
        service.end("");
        tsf.after_dispatch();
        assert_eq!(value.get(), "see 火@file.pdf日本");
        assert_eq!(atoms().len(), 1);

        // Over a selection from "see" into the atom: it goes whole.
        editor.set_selection(2, 12);
        tsf.after_dispatch();
        service.compose("z");
        service.end("Z");
        tsf.after_dispatch();
        let now = value.get();
        assert_eq!(
            now.matches("@fi").count(),
            now.matches("@file.pdf").count(),
            "never a piece of the atom: {now:?}"
        );
        assert!(
            atoms()
                .iter()
                .all(|a| now.get(a.range.clone()) == Some("@file.pdf")),
            "an atom left is whole: {now:?} {:?}",
            atoms()
        );
        drop(service);
        drop(tsf);
        // SAFETY: the window made above.
        let _ = unsafe { DestroyWindow(hwnd) };
    }

    #[test]
    fn query_only_insert_and_get_text_runs_obey_tsf_contract() {
        let mut ui = Ui::new(400., 200.);
        let value = ui.signal(String::from("12"));
        let view = ui.mount(column().child(text_input("Name", value).id("name")));
        ui.prepare_frame();
        let shared = Rc::new(RefCell::new(Shared {
            store: TextStore::new(),
            sink: None,
            sink_mask: 0,
            cancelled: false,
            lock: TS_LF_READ.0,
            pending: None,
            hwnd: HWND::default(),
            scale: 1.,
            told_len: 0,
            layout_changed: false,
            attributes: None,
            scope_asked: false,
            told_scope: 0,
        }));
        let store: ITextStoreACP2 = Store(shared.clone()).into();
        ui.input.focus(&ui.scene, view.find("name"));
        shared.borrow_mut().store.focus(ui.focused_editor());
        unsafe {
            // TF_IAS_QUERYONLY succeeds under a read-only lock.
            let (mut s, mut e) = (-1, -1);
            let mut change = TS_TEXTCHANGE::default();
            store
                .InsertTextAtSelection(TF_IAS_QUERYONLY.0, &[], &mut s, &mut e, &mut change)
                .unwrap();
            assert_eq!((s, e), (2, 2));

            // GetText runs-only call reports the plain run length and advances next.
            let mut runs = [TS_RUNINFO::default()];
            let mut text_given = 99;
            let mut runs_given = 0;
            let mut next = 0;
            store
                .GetText(
                    0,
                    -1,
                    &mut [],
                    &mut text_given,
                    &mut runs,
                    &mut runs_given,
                    &mut next,
                )
                .unwrap();
            assert_eq!(text_given, 0);
            assert_eq!(runs_given, 1);
            assert_eq!(runs[0].uCount, 2);
            assert_eq!(next, 2);

            // Out-of-bounds GetText and QueryInsert return TSF errors.
            assert_eq!(
                store
                    .GetText(
                        5,
                        -1,
                        &mut [],
                        &mut text_given,
                        &mut [],
                        &mut runs_given,
                        &mut next,
                    )
                    .unwrap_err()
                    .code(),
                TS_E_INVALIDPOS
            );
            assert_eq!(
                store
                    .QueryInsert(0, 99, 1, &mut s, &mut e)
                    .unwrap_err()
                    .code(),
                E_INVALIDARG
            );
        }
    }

    /// Whether the framework can start here: a service's session (a shell
    /// over SSH) has no input desktop, and it doesn't. Says so where not.
    fn interactive() -> bool {
        use windows::Win32::System::{
            RemoteDesktop::ProcessIdToSessionId, Threading::GetCurrentProcessId,
        };
        let mut session = 0;
        // SAFETY: this process's id; the out value is a local.
        let known = unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) }.is_ok();
        if known && session == 0 {
            eprintln!("skipped: session 0 has no desktop for the Text Services Framework");
            return false;
        }
        true
    }

    /// The framework's sink, what it is told and the locks it is granted
    /// noted on the way.
    #[implement(ITextStoreACPSink)]
    struct Recorder(Rc<RefCell<Vec<String>>>, ITextStoreACPSink);
    impl ITextStoreACPSink_Impl for Recorder_Impl {
        fn OnTextChange(
            &self,
            flags: TEXT_STORE_TEXT_CHANGE_FLAGS,
            change: *const TS_TEXTCHANGE,
        ) -> Result<()> {
            // SAFETY: the store's own change; the sink is the framework's.
            unsafe {
                let TS_TEXTCHANGE {
                    acpStart,
                    acpOldEnd,
                    acpNewEnd,
                } = *change;
                self.0
                    .borrow_mut()
                    .push(format!("text {acpStart}..{acpOldEnd} -> {acpNewEnd}"));
                self.1.OnTextChange(flags, change)
            }
        }
        fn OnSelectionChange(&self) -> Result<()> {
            // SAFETY: the framework's sink, as in each call below.
            unsafe { self.1.OnSelectionChange() }
        }
        fn OnLayoutChange(&self, code: TsLayoutCode, view: u32) -> Result<()> {
            unsafe { self.1.OnLayoutChange(code, view) }
        }
        fn OnStatusChange(&self, flags: u32) -> Result<()> {
            unsafe { self.1.OnStatusChange(flags) }
        }
        fn OnAttrsChange(
            &self,
            start: i32,
            end: i32,
            count: u32,
            attrs: *const GUID,
        ) -> Result<()> {
            unsafe {
                let attrs = std::slice::from_raw_parts(attrs, count as usize);
                self.1.OnAttrsChange(start, end, attrs)
            }
        }
        fn OnLockGranted(&self, flags: TEXT_STORE_LOCK_FLAGS) -> Result<()> {
            self.0.borrow_mut().push("lock".into());
            unsafe { self.1.OnLockGranted(flags) }
        }
        fn OnStartEditTransaction(&self) -> Result<()> {
            unsafe { self.1.OnStartEditTransaction() }
        }
        fn OnEndEditTransaction(&self) -> Result<()> {
            unsafe { self.1.OnEndEditTransaction() }
        }
    }

    /// A text area of `text`, focused, its caret at the end, with the
    /// framework in a hidden window, and what the framework is told: `run`.
    fn with_framework(
        text: &str,
        run: impl FnOnce(&EditorHandle, &Tsf, &Service, &RefCell<Vec<String>>),
    ) {
        use windows::Win32::{
            System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx},
            UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
            },
        };
        if !interactive() {
            return;
        }
        // SAFETY: COM on this test's thread (a single-threaded apartment).
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        // SAFETY: a plain window of a system class, never shown.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!(""),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                200,
                100,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        let mut ui = Ui::new(400., 200.);
        let value = ui.signal(text.to_owned());
        let view = ui.mount(text_area("Notes", value).size(300., 100.));
        ui.prepare_frame();
        ui.input.focus(&ui.scene, Some(view.node()));
        let editor = ui.focused_editor().unwrap();
        editor.set_selection(text.len(), text.len());
        let mut tsf = Tsf::new(hwnd, 1.).expect("the framework");
        tsf.focus(ui.focused_editor());
        tsf.after_dispatch();
        let told = Rc::new(RefCell::new(Vec::new()));
        let sink = tsf.shared.borrow().sink.clone();
        let sink = sink.expect("the framework advised its sink");
        tsf.shared.borrow_mut().sink = Some(Recorder(told.clone(), sink).into());
        let service = Service::new(&tsf);
        run(&editor, &tsf, &service, &told);
        drop((service, tsf));
        // SAFETY: the window made above.
        let _ = unsafe { DestroyWindow(hwnd) };
    }

    /// An input method reading what is before the caret may ask where it
    /// would insert, and what was typed is told as that, not as the whole
    /// document.
    #[test]
    fn a_digit_typed_is_told_as_itself_and_a_reader_may_query_an_insertion() {
        use windows::Win32::UI::TextServices::{ITfInsertAtSelection, TF_ES_READ};
        with_framework("ab", |editor, tsf, service, told| {
            editor.insert_text("1");
            tsf.after_dispatch();
            let seen = Rc::new(RefCell::new(String::new()));
            let (context, out) = (service.context.clone(), seen.clone());
            let edit: Edit = Box::new(move |cookie| {
                // SAFETY: within the session, with its cookie.
                unsafe {
                    context
                        .cast::<ITfInsertAtSelection>()?
                        .InsertTextAtSelection(cookie, TF_IAS_QUERYONLY, &[])?;
                    let (mut selection, mut fetched) = ([TF_SELECTION::default()], 0);
                    context.GetSelection(
                        cookie,
                        TF_DEFAULT_SELECTION,
                        &mut selection,
                        &mut fetched,
                    )?;
                    let [TF_SELECTION { range, .. }] = selection;
                    let range = std::mem::ManuallyDrop::into_inner(range).ok_or(E_FAIL)?;
                    let mut moved = 0;
                    range.ShiftStart(cookie, -2, &mut moved, std::ptr::null())?;
                    let (mut text, mut len) = ([0; 8], 0);
                    range.GetText(cookie, 0, &mut text, &mut len)?;
                    *out.borrow_mut() = String::from_utf16_lossy(&text[..len as usize]);
                }
                Ok(())
            });
            let session: ITfEditSession = Session(RefCell::new(Some(edit))).into();
            // SAFETY: zgx's context and the session, both alive.
            let done = unsafe {
                service.context.RequestEditSession(
                    service.client,
                    &session,
                    TF_ES_SYNC | TF_ES_READ,
                )
            };
            done.unwrap()
                .ok()
                .expect("a query-only insert in a read session");
            assert_eq!(*seen.borrow(), "b1");
            let told: Vec<_> = told.borrow().clone();
            assert_eq!(
                told.iter().filter(|t| *t != "lock").collect::<Vec<_>>(),
                ["text 2..2 -> 3"]
            );
        });
    }

    /// The app's edit, told as its turn ends, is told once and as it was,
    /// also where an input method read the document before that; and what it
    /// composes next goes after it.
    #[test]
    fn an_edit_told_before_an_input_method_composes_is_told_once() {
        use windows::Win32::UI::TextServices::TF_ES_READ;
        with_framework("ab", |editor, tsf, service, told| {
            let heard = || -> Vec<String> {
                let told = std::mem::take(&mut *told.borrow_mut());
                told.into_iter().filter(|t| t != "lock").collect()
            };
            editor.insert_text("1");
            let session: ITfEditSession = Session(RefCell::new(Some(Box::new(|_| Ok(()))))).into();
            // SAFETY: zgx's context and the session, both alive.
            unsafe {
                service.context.RequestEditSession(
                    service.client,
                    &session,
                    TF_ES_SYNC | TF_ES_READ,
                )
            }
            .unwrap()
            .ok()
            .unwrap();
            assert_eq!(heard(), [""; 0]);
            tsf.after_dispatch();
            assert_eq!(heard(), ["text 2..2 -> 3"]);
            service.compose("x");
            assert!(service.composition.borrow().is_some());
            tsf.after_dispatch();
            service.end("X");
            tsf.after_dispatch();
            assert_eq!(editor.editor.borrow().text(), "ab1X");
            assert_eq!(heard(), [""; 0], "its own edits aren't told back");
        });
    }

    /// An app's edit that cancels the open composition is told as its turn
    /// ends, the input method's composition ended with it, and it composes
    /// again.
    #[test]
    fn an_edit_that_cancels_a_composition_ends_the_input_methods() {
        with_framework("ab", |editor, tsf, service, told| {
            service.compose("x");
            assert!(service.composition.borrow().is_some());
            told.borrow_mut().clear();
            editor.replace_text(0..1, "Z");
            tsf.after_dispatch();
            // "abx" became "Zb": told whole, its composition beside it.
            assert_eq!(
                told.borrow().first().map(String::as_str),
                Some("text 0..3 -> 2")
            );
            assert!(service.composition.borrow().is_none(), "ended");
            assert!(!tsf.shared.borrow().store.is_composing());
            assert_eq!(editor.editor.borrow().text(), "Zb");
            editor.set_selection(0, 0);
            tsf.after_dispatch();
            service.compose("n");
            assert!(tsf.shared.borrow().store.is_composing());
            service.end("N");
            tsf.after_dispatch();
            assert_eq!(editor.editor.borrow().text(), "NZb");
        });
    }

    /// A sink that asks for a lock from inside `OnTextChange`, and edits under
    /// it: what it was granted, in order.
    #[implement(ITextStoreACPSink)]
    struct Asking {
        store: ITextStoreACP2,
        log: Rc<RefCell<Vec<String>>>,
        inside: std::cell::Cell<bool>,
    }
    impl ITextStoreACPSink_Impl for Asking_Impl {
        fn OnTextChange(
            &self,
            _flags: TEXT_STORE_TEXT_CHANGE_FLAGS,
            change: *const TS_TEXTCHANGE,
        ) -> Result<()> {
            // SAFETY: the store's own change, and its own method.
            unsafe {
                let TS_TEXTCHANGE {
                    acpStart,
                    acpOldEnd,
                    acpNewEnd,
                } = *change;
                self.log
                    .borrow_mut()
                    .push(format!("text {acpStart}..{acpOldEnd} -> {acpNewEnd}"));
                self.inside.set(true);
                let asked = self.store.RequestLock(TS_LF_READWRITE.0 | TS_LF_SYNC);
                self.inside.set(false);
                self.log.borrow_mut().push(format!("asked {:?}", asked?.0));
            }
            Ok(())
        }
        fn OnSelectionChange(&self) -> Result<()> {
            Ok(())
        }
        fn OnLayoutChange(&self, _code: TsLayoutCode, _view: u32) -> Result<()> {
            Ok(())
        }
        fn OnStatusChange(&self, _flags: u32) -> Result<()> {
            Ok(())
        }
        fn OnAttrsChange(&self, _: i32, _: i32, _: u32, _: *const GUID) -> Result<()> {
            Ok(())
        }
        fn OnLockGranted(&self, _flags: TEXT_STORE_LOCK_FLAGS) -> Result<()> {
            let what = match self.inside.get() {
                true => "lock inside",
                false => "lock",
            };
            self.log.borrow_mut().push(what.into());
            if self.inside.get() {
                let text: Vec<u16> = "!".encode_utf16().collect();
                let (mut a, mut b, mut change) = (0, 0, TS_TEXTCHANGE::default());
                // SAFETY: the store's own method, under its lock.
                unsafe {
                    self.store
                        .InsertTextAtSelection(0, &text, &mut a, &mut b, &mut change)?;
                }
            }
            Ok(())
        }
        fn OnStartEditTransaction(&self) -> Result<()> {
            Ok(())
        }
        fn OnEndEditTransaction(&self) -> Result<()> {
            Ok(())
        }
    }

    /// Told of an edit, a sink may ask for a lock there and edit: it is
    /// granted and given back, and each edit told once.
    #[test]
    fn a_sink_being_told_may_ask_for_a_lock_and_edit() {
        let mut ui = Ui::new(400., 200.);
        let value = ui.signal(String::from("ab"));
        let view = ui.mount(text_area("Notes", value.clone()).size(300., 100.));
        ui.prepare_frame();
        ui.input.focus(&ui.scene, Some(view.node()));
        let editor = ui.focused_editor().unwrap();
        editor.set_selection(2, 2);
        let shared = Rc::new(RefCell::new(Shared {
            store: TextStore::new(),
            sink: None,
            sink_mask: TS_AS_TEXT_CHANGE | TS_AS_SEL_CHANGE,
            cancelled: false,
            lock: 0,
            pending: None,
            hwnd: HWND::default(),
            scale: 1.,
            told_len: 0,
            layout_changed: false,
            attributes: None,
            scope_asked: false,
            told_scope: 0,
        }));
        shared.borrow_mut().store.focus(ui.focused_editor());
        shared.borrow_mut().told_len = 2;
        let store: ITextStoreACP2 = Store(shared.clone()).into();
        let log = Rc::new(RefCell::new(Vec::new()));
        shared.borrow_mut().sink = Some(
            Asking {
                store: store.clone(),
                log: log.clone(),
                inside: Default::default(),
            }
            .into(),
        );
        editor.insert_text("1");
        tell(&shared);
        let heard = || std::mem::take(&mut *log.borrow_mut());
        assert_eq!(heard(), ["text 2..2 -> 3", "lock inside", "asked 0"]);
        assert_eq!(shared.borrow().lock, 0);
        tell(&shared);
        assert_eq!(heard(), [""; 0], "its own edit isn't told back");
        assert_eq!(value.get(), "ab1!");
        shared.borrow_mut().sink = None;
    }
}
