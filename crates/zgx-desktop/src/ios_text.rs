//! The system's text input over zgx's editors: a `UITextInput` responder the
//! keyboard talks to, so autocorrect, dictation, marked text (Japanese,
//! Chinese, Korean), the keyboard's cursor and its suggestions are UIKit's, and
//! zgx draws. What it reads and edits is `TextStore`, the document Windows'
//! input methods use: an editor's text with its composition in place, in the
//! UTF-16 offsets both count in.
//!
//! The classes are made at run time: this objc2 has no `UITextPosition`,
//! `UITextRange` or `UIView` to subclass.
use crate::text_store::{Change, TextStore};
use objc2::{
    ClassType, class, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, AnyProtocol, Bool, ClassBuilder, NSObject, Sel},
    sel,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, OnceLock},
};
use unicode_segmentation::UnicodeSegmentation;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use zgx::{text_edit::InputMode, widgets::EditorHandle};

/// What the responder works on.
struct State {
    store: RefCell<TextStore>,
    window: Arc<winit::window::Window>,
    /// UIKit's watcher of this input, told of what changes behind its back.
    delegate: Cell<*mut AnyObject>,
    /// The marked text's selection (UTF-16, within it).
    tokenizer: RefCell<Option<Retained<AnyObject>>>,
    /// Return was pressed: the host dispatches it as a key.
    enter: Cell<bool>,
    /// A tap, as the host saw it, for the keyboard's own cursor.
    told_len: Cell<usize>,
    /// Where the view is, on the focused editor: what UIKit asks and tells of
    /// text is relative to it, and its gestures are over it.
    origin: Cell<(f32, f32)>,
    /// An editor has the keyboard: touches on it are `UITextInteraction`'s.
    editing: Cell<bool>,
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<State>>> = const { RefCell::new(None) };
}
fn state() -> Option<Rc<State>> {
    CURRENT.with(|current| current.borrow().clone())
}

fn nsstring(text: &str) -> Retained<AnyObject> {
    let text = std::ffi::CString::new(text.replace('\0', "")).unwrap_or_default();
    // SAFETY: NSString's constructor, over a NUL-terminated string.
    unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
}
fn from_nsstring(text: *mut AnyObject) -> String {
    if text.is_null() {
        return String::new();
    }
    // SAFETY: an NSString from UIKit; its UTF-8 view is copied at once.
    unsafe {
        let bytes: *const std::ffi::c_char = msg_send![text, UTF8String];
        if bytes.is_null() {
            return String::new();
        }
        std::ffi::CStr::from_ptr(bytes)
            .to_string_lossy()
            .into_owned()
    }
}

// Positions and ranges: offsets, in objects UIKit can hold.

fn position_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let mut builder =
            ClassBuilder::new(c"ZgxTextPosition", class!(UITextPosition)).expect("one class");
        builder.add_ivar::<usize>(c"offset");
        builder.register()
    })
}
fn position(offset: usize) -> *mut AnyObject {
    // SAFETY: a new instance of the class above, handed to UIKit autoreleased.
    unsafe {
        let made: Retained<AnyObject> = msg_send![position_class(), new];
        let ivar = position_class()
            .instance_variable(c"offset")
            .expect("declared");
        *ivar.load_ptr::<usize>(&made) = offset;
        Retained::autorelease_return(made)
    }
}
fn offset_of(position: *mut AnyObject) -> usize {
    if position.is_null() {
        return 0;
    }
    // SAFETY: UIKit gives back positions this file made.
    unsafe {
        let ivar = position_class()
            .instance_variable(c"offset")
            .expect("declared");
        *ivar.load::<usize>(&*position)
    }
}

fn range_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    extern "C-unwind" fn start(this: &AnyObject, _: Sel) -> *mut AnyObject {
        position(ends(this).0)
    }
    extern "C-unwind" fn end(this: &AnyObject, _: Sel) -> *mut AnyObject {
        position(ends(this).1)
    }
    extern "C-unwind" fn is_empty(this: &AnyObject, _: Sel) -> Bool {
        let (start, end) = ends(this);
        Bool::new(start == end)
    }
    CLASS.get_or_init(|| {
        let mut builder =
            ClassBuilder::new(c"ZgxTextRange", class!(UITextRange)).expect("one class");
        builder.add_ivar::<usize>(c"from");
        builder.add_ivar::<usize>(c"to");
        // SAFETY: each has the signature of the getter it answers.
        unsafe {
            builder.add_method(sel!(start), start as extern "C-unwind" fn(_, _) -> _);
            builder.add_method(sel!(end), end as extern "C-unwind" fn(_, _) -> _);
            builder.add_method(sel!(isEmpty), is_empty as extern "C-unwind" fn(_, _) -> _);
        }
        builder.register()
    })
}
fn range(start: usize, end: usize) -> *mut AnyObject {
    // SAFETY: a new instance of the class above, handed to UIKit autoreleased.
    unsafe {
        let made: Retained<AnyObject> = msg_send![range_class(), new];
        *range_class()
            .instance_variable(c"from")
            .expect("declared")
            .load_ptr::<usize>(&made) = start;
        *range_class()
            .instance_variable(c"to")
            .expect("declared")
            .load_ptr::<usize>(&made) = end;
        Retained::autorelease_return(made)
    }
}
fn ends(range: &AnyObject) -> (usize, usize) {
    // SAFETY: UIKit gives back ranges this file made.
    unsafe {
        (
            *range_class()
                .instance_variable(c"from")
                .expect("declared")
                .load::<usize>(range),
            *range_class()
                .instance_variable(c"to")
                .expect("declared")
                .load::<usize>(range),
        )
    }
}
fn range_ends(range: *mut AnyObject) -> (usize, usize) {
    if range.is_null() {
        return (0, 0);
    }
    // SAFETY: not null, and one of this file's.
    ends(unsafe { &*range })
}

/// What's told to UIKit: `name` with this input, around an edit of its own.
fn tell(state: &State, this: &AnyObject, will: Sel, did: Sel, edit: impl FnOnce()) {
    let delegate = state.delegate.get();
    // SAFETY: UITextInputDelegate's methods on UIKit's object, set by UIKit.
    unsafe {
        if !delegate.is_null() {
            let _: *mut AnyObject = msg_send![&*delegate, performSelector: will, withObject: this];
        }
        edit();
        if !delegate.is_null() {
            let _: *mut AnyObject = msg_send![&*delegate, performSelector: did, withObject: this];
        }
    }
}

// The responder's methods. Each runs on the main thread, called by UIKit.

extern "C-unwind" fn yes(_: &AnyObject, _: Sel) -> Bool {
    Bool::YES
}
extern "C-unwind" fn has_text(_: &AnyObject, _: Sel) -> Bool {
    Bool::new(state().is_some_and(|state| state.store.borrow().len() > 0))
}
extern "C-unwind" fn insert_text(_: &AnyObject, _: Sel, text: *mut AnyObject) {
    let Some(state) = state() else { return };
    let text = from_nsstring(text);
    // Return is a key: a single-line field submits, and an editor breaks its line.
    if text == "\n" {
        state.enter.set(true);
        state.window.request_redraw();
        return;
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut store = state.store.borrow_mut();
    if store.is_composing() {
        store.end_composition();
    }
    let (start, end, _) = store.selection();
    store.replace(start, end, &units);
    drop(store);
    state.window.request_redraw();
}
extern "C-unwind" fn delete_backward(_: &AnyObject, _: Sel) {
    let Some(state) = state() else { return };
    let mut store = state.store.borrow_mut();
    let (start, end, _) = store.selection();
    if start != end {
        store.replace(start, end, &[]);
    } else if start > 0 {
        // Back one grapheme, as a field's delete key goes: an emoji whole.
        let before = String::from_utf16_lossy(&store.text(0, start));
        let last = before
            .graphemes(true)
            .next_back()
            .map_or(0, |g| g.encode_utf16().count());
        store.replace(start - last, start, &[]);
    }
    drop(store);
    state.window.request_redraw();
}
extern "C-unwind" fn text_in_range(_: &AnyObject, _: Sel, range: *mut AnyObject) -> *mut AnyObject {
    let Some(state) = state() else {
        return std::ptr::null_mut();
    };
    let (start, end) = range_ends(range);
    let text = String::from_utf16_lossy(&state.store.borrow().text(start, end));
    Retained::autorelease_return(nsstring(&text))
}
extern "C-unwind" fn replace_range(
    _: &AnyObject,
    _: Sel,
    range: *mut AnyObject,
    text: *mut AnyObject,
) {
    let Some(state) = state() else { return };
    let (start, end) = range_ends(range);
    let units: Vec<u16> = from_nsstring(text).encode_utf16().collect();
    state.store.borrow_mut().replace(start, end, &units);
    state.window.request_redraw();
}
extern "C-unwind" fn selected_text_range(_: &AnyObject, _: Sel) -> *mut AnyObject {
    let Some(state) = state() else {
        return std::ptr::null_mut();
    };
    let (start, end, _) = state.store.borrow().selection();
    range(start, end)
}
extern "C-unwind" fn set_selected_text_range(_: &AnyObject, _: Sel, new: *mut AnyObject) {
    let Some(state) = state() else { return };
    let (start, end) = range_ends(new);
    state.store.borrow_mut().set_selection(start, end, false);
    state.window.request_redraw();
}
extern "C-unwind" fn marked_text_range(_: &AnyObject, _: Sel) -> *mut AnyObject {
    let Some(state) = state() else {
        return std::ptr::null_mut();
    };
    let store = state.store.borrow();
    match store.composition_range() {
        Some(marked) => range(marked.start, marked.end),
        None => std::ptr::null_mut(),
    }
}
extern "C-unwind" fn marked_text_style(_: &AnyObject, _: Sel) -> *mut AnyObject {
    std::ptr::null_mut()
}
extern "C-unwind" fn set_marked_text_style(_: &AnyObject, _: Sel, _: *mut AnyObject) {}
#[repr(C)]
#[derive(Clone, Copy)]
struct NSRange {
    location: usize,
    length: usize,
}
// SAFETY: Foundation's `NSRange`.
unsafe impl objc2::Encode for NSRange {
    const ENCODING: objc2::Encoding =
        objc2::Encoding::Struct("_NSRange", &[usize::ENCODING, usize::ENCODING]);
}
// SAFETY: as above.
unsafe impl objc2::RefEncode for NSRange {
    const ENCODING_REF: objc2::Encoding =
        objc2::Encoding::Pointer(&<Self as objc2::Encode>::ENCODING);
}
extern "C-unwind" fn set_marked_text(
    _: &AnyObject,
    _: Sel,
    text: *mut AnyObject,
    selected: NSRange,
) {
    let Some(state) = state() else { return };
    let units: Vec<u16> = from_nsstring(text).encode_utf16().collect();
    let mut store = state.store.borrow_mut();
    if !store.is_composing() {
        let (start, end, _) = store.selection();
        store.start_composition(start..end);
    }
    if let Some(marked) = store.composition_range() {
        let at = marked.start;
        store.replace(marked.start, marked.end, &units);
        let caret = at + selected.location.min(units.len());
        store.set_selection(caret, caret + selected.length, false);
    }
    drop(store);
    state.window.request_redraw();
}
extern "C-unwind" fn unmark_text(_: &AnyObject, _: Sel) {
    let Some(state) = state() else { return };
    state.store.borrow_mut().end_composition();
    state.window.request_redraw();
}
extern "C-unwind" fn beginning_of_document(_: &AnyObject, _: Sel) -> *mut AnyObject {
    position(0)
}
extern "C-unwind" fn end_of_document(_: &AnyObject, _: Sel) -> *mut AnyObject {
    position(state().map_or(0, |state| state.store.borrow().len()))
}
extern "C-unwind" fn range_from(
    _: &AnyObject,
    _: Sel,
    a: *mut AnyObject,
    b: *mut AnyObject,
) -> *mut AnyObject {
    let (a, b) = (offset_of(a), offset_of(b));
    range(a.min(b), a.max(b))
}
extern "C-unwind" fn position_offset(
    _: &AnyObject,
    _: Sel,
    from: *mut AnyObject,
    by: isize,
) -> *mut AnyObject {
    let len = state().map_or(0, |state| state.store.borrow().len()) as isize;
    let at = offset_of(from) as isize + by;
    if at < 0 || at > len {
        return std::ptr::null_mut();
    }
    position(at as usize)
}
extern "C-unwind" fn position_direction(
    this: &AnyObject,
    sel: Sel,
    from: *mut AnyObject,
    direction: isize,
    by: isize,
) -> *mut AnyObject {
    // UITextLayoutDirection: right 2 and down 5 go forward, left 3 and up 4 back.
    let by = if matches!(direction, 3 | 4) { -by } else { by };
    position_offset(this, sel, from, by)
}
extern "C-unwind" fn compare(_: &AnyObject, _: Sel, a: *mut AnyObject, b: *mut AnyObject) -> isize {
    match offset_of(a).cmp(&offset_of(b)) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}
extern "C-unwind" fn offset_between(
    _: &AnyObject,
    _: Sel,
    a: *mut AnyObject,
    b: *mut AnyObject,
) -> isize {
    offset_of(b) as isize - offset_of(a) as isize
}
extern "C-unwind" fn input_delegate(_: &AnyObject, _: Sel) -> *mut AnyObject {
    state().map_or(std::ptr::null_mut(), |state| state.delegate.get())
}
extern "C-unwind" fn set_input_delegate(_: &AnyObject, _: Sel, delegate: *mut AnyObject) {
    if let Some(state) = state() {
        state.delegate.set(delegate);
    }
}
extern "C-unwind" fn tokenizer(this: &AnyObject, _: Sel) -> *mut AnyObject {
    let Some(state) = state() else {
        return std::ptr::null_mut();
    };
    let mut slot = state.tokenizer.borrow_mut();
    let made = slot.get_or_insert_with(|| {
        // SAFETY: UIKit's own tokenizer, over this input.
        unsafe {
            msg_send![msg_send![class!(UITextInputStringTokenizer), alloc], initWithTextInput: this]
        }
    });
    Retained::as_ptr(made) as *mut AnyObject
}
extern "C-unwind" fn farthest(
    _: &AnyObject,
    _: Sel,
    within: *mut AnyObject,
    direction: isize,
) -> *mut AnyObject {
    let (start, end) = range_ends(within);
    position(if matches!(direction, 3 | 4) {
        start
    } else {
        end
    })
}
extern "C-unwind" fn extending(
    _: &AnyObject,
    _: Sel,
    from: *mut AnyObject,
    direction: isize,
) -> *mut AnyObject {
    let at = offset_of(from);
    let len = state().map_or(0, |state| state.store.borrow().len());
    if matches!(direction, 3 | 4) {
        range(0, at)
    } else {
        range(at, len)
    }
}
extern "C-unwind" fn writing_direction(
    _: &AnyObject,
    _: Sel,
    _: *mut AnyObject,
    _: isize,
) -> isize {
    // NSWritingDirectionNatural.
    -1
}
extern "C-unwind" fn set_writing_direction(_: &AnyObject, _: Sel, _: isize, _: *mut AnyObject) {}
fn area(start: usize, end: usize) -> CGRect {
    let found = state().and_then(|state| state.store.borrow().area(start, end));
    found.map_or(
        CGRect {
            origin: CGPoint { x: 0., y: 0. },
            size: CGSize {
                width: 0.,
                height: 0.,
            },
        },
        in_view,
    )
}
/// A rectangle in the window's points, in the view's.
fn in_view(r: zgx::scene::Rect) -> CGRect {
    let (x, y) = state().map_or((0., 0.), |state| state.origin.get());
    CGRect {
        origin: CGPoint {
            x: (r.x - x) as f64,
            y: (r.y - y) as f64,
        },
        size: CGSize {
            width: r.width as f64,
            height: r.height as f64,
        },
    }
}
/// The document's offset nearest a point in the view.
fn offset_at_point(point: CGPoint) -> Option<usize> {
    let state = state()?;
    let (x, y) = state.origin.get();
    state
        .store
        .borrow()
        .offset_at(point.x as f32 + x, point.y as f32 + y)
}
/// `UITextSelectionRect`: one line's box of a selection.
fn selection_rect_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    fn ivar(this: &AnyObject, name: &std::ffi::CStr) -> f64 {
        let class = selection_rect_class();
        // SAFETY: an instance of this class, which declares it.
        unsafe {
            *class
                .instance_variable(name)
                .expect("declared")
                .load::<f64>(this)
        }
    }
    extern "C-unwind" fn get_rect(this: &AnyObject, _: Sel) -> CGRect {
        CGRect {
            origin: CGPoint {
                x: ivar(this, c"x"),
                y: ivar(this, c"y"),
            },
            size: CGSize {
                width: ivar(this, c"w"),
                height: ivar(this, c"h"),
            },
        }
    }
    extern "C-unwind" fn writing_direction(_: &AnyObject, _: Sel) -> isize {
        // NSWritingDirectionNatural.
        -1
    }
    extern "C-unwind" fn contains_start(this: &AnyObject, _: Sel) -> Bool {
        Bool::new(ivar(this, c"first") != 0.)
    }
    extern "C-unwind" fn contains_end(this: &AnyObject, _: Sel) -> Bool {
        Bool::new(ivar(this, c"last") != 0.)
    }
    extern "C-unwind" fn is_vertical(_: &AnyObject, _: Sel) -> Bool {
        Bool::NO
    }
    CLASS.get_or_init(|| {
        let mut builder =
            ClassBuilder::new(c"ZgxSelectionRect", class!(UITextSelectionRect)).expect("one class");
        for name in [c"x", c"y", c"w", c"h", c"first", c"last"] {
            builder.add_ivar::<f64>(name);
        }
        // SAFETY: each has the signature of the selector it answers.
        unsafe {
            builder.add_method(sel!(rect), get_rect as extern "C-unwind" fn(_, _) -> _);
            builder.add_method(
                sel!(writingDirection),
                writing_direction as extern "C-unwind" fn(_, _) -> _,
            );
            builder.add_method(
                sel!(containsStart),
                contains_start as extern "C-unwind" fn(_, _) -> _,
            );
            builder.add_method(
                sel!(containsEnd),
                contains_end as extern "C-unwind" fn(_, _) -> _,
            );
            builder.add_method(
                sel!(isVertical),
                is_vertical as extern "C-unwind" fn(_, _) -> _,
            );
        }
        builder.register()
    })
}
extern "C-unwind" fn first_rect(_: &AnyObject, _: Sel, range: *mut AnyObject) -> CGRect {
    let (start, end) = range_ends(range);
    area(start, end)
}
extern "C-unwind" fn caret_rect(_: &AnyObject, _: Sel, at: *mut AnyObject) -> CGRect {
    let at = offset_of(at);
    let mut rect = area(at, at);
    rect.size.width = rect.size.width.max(2.);
    rect
}
extern "C-unwind" fn selection_rects(
    _: &AnyObject,
    _: Sel,
    range: *mut AnyObject,
) -> *mut AnyObject {
    let (start, end) = range_ends(range);
    let lines = state().map_or(Vec::new(), |state| {
        state.store.borrow().line_areas(start, end)
    });
    let class = selection_rect_class();
    let count = lines.len();
    // SAFETY: new instances of the class above, their ivars set, in an array
    // handed to UIKit autoreleased.
    unsafe {
        let array: Retained<AnyObject> = msg_send![class!(NSMutableArray), array];
        for (index, line) in lines.into_iter().enumerate() {
            let rect = in_view(line);
            let made: Retained<AnyObject> = msg_send![class, new];
            let set = |name: &std::ffi::CStr, value: f64| {
                *class
                    .instance_variable(name)
                    .expect("declared")
                    .load_ptr::<f64>(&made) = value;
            };
            set(c"x", rect.origin.x);
            set(c"y", rect.origin.y);
            set(c"w", rect.size.width);
            set(c"h", rect.size.height);
            set(c"first", f64::from(u8::from(index == 0)));
            set(c"last", f64::from(u8::from(index + 1 == count)));
            let _: () = msg_send![&*array, addObject: &*made];
        }
        Retained::autorelease_return(array)
    }
}
extern "C-unwind" fn closest_position(_: &AnyObject, _: Sel, at: CGPoint) -> *mut AnyObject {
    let found = offset_at_point(at);
    position(found.unwrap_or_else(|| state().map_or(0, |state| state.store.borrow().selection().1)))
}
extern "C-unwind" fn closest_within(
    _: &AnyObject,
    _: Sel,
    at: CGPoint,
    within: *mut AnyObject,
) -> *mut AnyObject {
    let (start, end) = range_ends(within);
    position(offset_at_point(at).unwrap_or(end).clamp(start, end))
}
extern "C-unwind" fn character_range(_: &AnyObject, _: Sel, at: CGPoint) -> *mut AnyObject {
    let Some(state) = state() else {
        return std::ptr::null_mut();
    };
    let Some(offset) = offset_at_point(at) else {
        return std::ptr::null_mut();
    };
    let len = state.store.borrow().len();
    range(offset.min(len), (offset + 1).min(len))
}
extern "C-unwind" fn text_input_view(this: &AnyObject, _: Sel) -> *const AnyObject {
    this
}

// UIResponderStandardEditActions: the edit menu's, and a hardware keyboard's.
extern "C-unwind" fn edit_copy(_: &AnyObject, _: Sel, _: *mut AnyObject) {
    let Some(state) = state() else { return };
    let editor = state.store.borrow().editor().cloned();
    if let Some(editor) = editor.filter(|editor| !editor.is_secure()) {
        let _ = crate::ios_pasteboard::Clipboard.set_text(editor.copy());
    }
}
extern "C-unwind" fn edit_cut(_: &AnyObject, _: Sel, _: *mut AnyObject) {
    let Some(state) = state() else { return };
    let editor = state.store.borrow().editor().cloned();
    if let Some(editor) = editor.filter(|editor| !editor.is_secure()) {
        let _ = crate::ios_pasteboard::Clipboard.set_text(editor.cut());
        state.window.request_redraw();
    }
}
extern "C-unwind" fn edit_paste(_: &AnyObject, _: Sel, _: *mut AnyObject) {
    let Some(state) = state() else { return };
    let editor = state.store.borrow().editor().cloned();
    if let (Some(editor), Ok(text)) = (editor, crate::ios_pasteboard::Clipboard.get_text()) {
        editor.paste(&text);
        state.window.request_redraw();
    }
}
extern "C-unwind" fn edit_select_all(_: &AnyObject, _: Sel, _: *mut AnyObject) {
    let Some(state) = state() else { return };
    if let Some(editor) = state.store.borrow().editor() {
        editor.select_all();
    }
    state.window.request_redraw();
}
extern "C-unwind" fn can_perform(this: &AnyObject, _: Sel, action: Sel, _: *mut AnyObject) -> Bool {
    let Some(state) = state() else {
        return Bool::NO;
    };
    let store = state.store.borrow();
    let (start, end, _) = store.selection();
    let selected = start != end;
    let secure = store.editor().is_some_and(EditorHandle::is_secure);
    let can = if action == sel!(copy:) || action == sel!(cut:) {
        selected && !secure
    } else if action == sel!(paste:) {
        // SAFETY: UIPasteboard's getter, which reads no contents and so shows
        // no prompt.
        unsafe {
            let board: Retained<AnyObject> = msg_send![class!(UIPasteboard), generalPasteboard];
            msg_send![&*board, hasStrings]
        }
    } else if action == sel!(selectAll:) {
        store.len() > 0
    } else {
        let _ = this;
        false
    };
    Bool::new(can)
}

fn input_mode() -> InputMode {
    state()
        .and_then(|state| state.store.borrow().editor().map(EditorHandle::input_mode))
        .unwrap_or_default()
}
extern "C-unwind" fn keyboard_type(_: &AnyObject, _: Sel) -> isize {
    // UIKeyboardType.
    match input_mode() {
        InputMode::Decimal => 8,
        InputMode::Numeric => 4,
        InputMode::Tel => 5,
        InputMode::Email => 7,
        InputMode::Url => 3,
        InputMode::Search => 10,
        _ => 0,
    }
}
extern "C-unwind" fn return_key_type(_: &AnyObject, _: Sel) -> isize {
    // UIReturnKeyType: search 6, else the default.
    if input_mode() == InputMode::Search {
        6
    } else {
        0
    }
}
extern "C-unwind" fn autocapitalization(_: &AnyObject, _: Sel) -> isize {
    // UITextAutocapitalizationType: none for addresses, else sentences.
    match input_mode() {
        InputMode::Email | InputMode::Url => 0,
        _ => 2,
    }
}
extern "C-unwind" fn is_secure(_: &AnyObject, _: Sel) -> Bool {
    Bool::new(state().is_some_and(|state| {
        state
            .store
            .borrow()
            .editor()
            .is_some_and(EditorHandle::is_secure)
    }))
}
/// Touches on the editor with the keyboard are `UITextInteraction`'s: its
/// cursor, handles, loupe and edit menu. Anywhere else, or with none, zgx's.
extern "C-unwind" fn hit_test(
    this: &AnyObject,
    _: Sel,
    at: CGPoint,
    event: *mut AnyObject,
) -> *mut AnyObject {
    if !state().is_some_and(|state| state.editing.get()) {
        return std::ptr::null_mut();
    }
    // SAFETY: UIView's method, UIKit asking on the main thread.
    let inside: Bool = unsafe { msg_send![this, pointInside: at, withEvent: event] };
    if inside.as_bool() {
        (this as *const AnyObject).cast_mut()
    } else {
        std::ptr::null_mut()
    }
}

fn input_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let mut b = ClassBuilder::new(c"ZgxTextInput", class!(UIView)).expect("one class");
        if let Some(protocol) = AnyProtocol::get(c"UITextInput") {
            b.add_protocol(protocol);
        }
        // SAFETY: each function has the signature of the selector it answers
        // (UIKeyInput's, UITextInput's, UITextInputTraits').
        unsafe {
            b.add_method(
                sel!(canBecomeFirstResponder),
                yes as extern "C-unwind" fn(_, _) -> _,
            );
            b.add_method(sel!(hasText), has_text as extern "C-unwind" fn(_, _) -> _);
            b.add_method(
                sel!(insertText:),
                insert_text as extern "C-unwind" fn(_, _, _),
            );
            b.add_method(
                sel!(deleteBackward),
                delete_backward as extern "C-unwind" fn(_, _),
            );
            b.add_method(
                sel!(textInRange:),
                text_in_range as extern "C-unwind" fn(_, _, _) -> _,
            );
            b.add_method(
                sel!(replaceRange:withText:),
                replace_range as extern "C-unwind" fn(_, _, _, _),
            );
            b.add_method(
                sel!(selectedTextRange),
                selected_text_range as extern "C-unwind" fn(_, _) -> _,
            );
            b.add_method(
                sel!(setSelectedTextRange:),
                set_selected_text_range as extern "C-unwind" fn(_, _, _),
            );
            b.add_method(
                sel!(markedTextRange),
                marked_text_range as extern "C-unwind" fn(_, _) -> _,
            );
            b.add_method(
                sel!(markedTextStyle),
                marked_text_style as extern "C-unwind" fn(_, _) -> _,
            );
            b.add_method(
                sel!(setMarkedTextStyle:),
                set_marked_text_style as extern "C-unwind" fn(_, _, _),
            );
            b.add_method(
                sel!(setMarkedText:selectedRange:),
                set_marked_text as extern "C-unwind" fn(_, _, _, _),
            );
            b.add_method(sel!(unmarkText), unmark_text as extern "C-unwind" fn(_, _));
            b.add_method(
                sel!(beginningOfDocument),
                beginning_of_document as extern "C-unwind" fn(_, _) -> _,
            );
            b.add_method(
                sel!(endOfDocument),
                end_of_document as extern "C-unwind" fn(_, _) -> _,
            );
            b.add_method(
                sel!(textRangeFromPosition:toPosition:),
                range_from as extern "C-unwind" fn(_, _, _, _) -> _,
            );
            b.add_method(
                sel!(positionFromPosition:offset:),
                position_offset as extern "C-unwind" fn(_, _, _, _) -> _,
            );
            b.add_method(
                sel!(positionFromPosition:inDirection:offset:),
                position_direction as extern "C-unwind" fn(_, _, _, _, _) -> _,
            );
            b.add_method(
                sel!(comparePosition:toPosition:),
                compare as extern "C-unwind" fn(_, _, _, _) -> _,
            );
            b.add_method(
                sel!(offsetFromPosition:toPosition:),
                offset_between as extern "C-unwind" fn(_, _, _, _) -> _,
            );
            b.add_method(
                sel!(inputDelegate),
                input_delegate as extern "C-unwind" fn(_, _) -> _,
            );
            b.add_method(
                sel!(setInputDelegate:),
                set_input_delegate as extern "C-unwind" fn(_, _, _),
            );
            b.add_method(
                sel!(tokenizer),
                tokenizer as extern "C-unwind" fn(_, _) -> _,
            );
            b.add_method(
                sel!(positionWithinRange:farthestInDirection:),
                farthest as extern "C-unwind" fn(_, _, _, _) -> _,
            );
            b.add_method(
                sel!(characterRangeByExtendingPosition:inDirection:),
                extending as extern "C-unwind" fn(_, _, _, _) -> _,
            );
            b.add_method(
                sel!(baseWritingDirectionForPosition:inDirection:),
                writing_direction as extern "C-unwind" fn(_, _, _, _) -> _,
            );
            b.add_method(
                sel!(setBaseWritingDirection:forRange:),
                set_writing_direction as extern "C-unwind" fn(_, _, _, _),
            );
            b.add_method(
                sel!(firstRectForRange:),
                first_rect as extern "C-unwind" fn(_, _, _) -> _,
            );
            b.add_method(
                sel!(caretRectForPosition:),
                caret_rect as extern "C-unwind" fn(_, _, _) -> _,
            );
            b.add_method(
                sel!(selectionRectsForRange:),
                selection_rects as extern "C-unwind" fn(_, _, _) -> _,
            );
            b.add_method(
                sel!(closestPositionToPoint:),
                closest_position as extern "C-unwind" fn(_, _, _) -> _,
            );
            b.add_method(
                sel!(closestPositionToPoint:withinRange:),
                closest_within as extern "C-unwind" fn(_, _, _, _) -> _,
            );
            b.add_method(
                sel!(characterRangeAtPoint:),
                character_range as extern "C-unwind" fn(_, _, _) -> _,
            );
            b.add_method(
                sel!(keyboardType),
                keyboard_type as extern "C-unwind" fn(_, _) -> _,
            );
            b.add_method(
                sel!(returnKeyType),
                return_key_type as extern "C-unwind" fn(_, _) -> _,
            );
            b.add_method(
                sel!(autocapitalizationType),
                autocapitalization as extern "C-unwind" fn(_, _) -> _,
            );
            b.add_method(
                sel!(isSecureTextEntry),
                is_secure as extern "C-unwind" fn(_, _) -> _,
            );
            b.add_method(sel!(copy:), edit_copy as extern "C-unwind" fn(_, _, _));
            b.add_method(sel!(cut:), edit_cut as extern "C-unwind" fn(_, _, _));
            b.add_method(sel!(paste:), edit_paste as extern "C-unwind" fn(_, _, _));
            b.add_method(
                sel!(selectAll:),
                edit_select_all as extern "C-unwind" fn(_, _, _),
            );
            b.add_method(
                sel!(canPerformAction:withSender:),
                can_perform as extern "C-unwind" fn(_, _, _, _) -> _,
            );
            b.add_method(
                sel!(hitTest:withEvent:),
                hit_test as extern "C-unwind" fn(_, _, _, _) -> _,
            );
            b.add_method(
                sel!(textInputView),
                text_input_view as extern "C-unwind" fn(_, _) -> _,
            );
        }
        b.register()
    })
}

/// The window's text input.
pub(crate) struct TextInput {
    view: Retained<AnyObject>,
    state: Rc<State>,
    focused: Cell<bool>,
    size: Cell<(f32, f32)>,
}

impl TextInput {
    pub(crate) fn new(window: Arc<winit::window::Window>) -> Option<Self> {
        let handle = window.window_handle().ok()?;
        let RawWindowHandle::UiKit(handle) = handle.as_raw() else {
            return None;
        };
        let state = Rc::new(State {
            store: RefCell::new(TextStore::new()),
            window: window.clone(),
            delegate: Cell::new(std::ptr::null_mut()),
            tokenizer: RefCell::default(),
            enter: Cell::new(false),
            told_len: Cell::new(0),
            origin: Cell::new((0., 0.)),
            editing: Cell::new(false),
        });
        CURRENT.with(|current| *current.borrow_mut() = Some(state.clone()));
        // SAFETY: winit's view, alive with its window; UIKit's calls on the
        // main thread, where windows are made.
        let view = unsafe {
            let root: *const AnyObject = handle.ui_view.as_ptr().cast();
            let view: Retained<AnyObject> = msg_send![input_class(), new];
            let _: () = msg_send![&*view, setAccessibilityElementsHidden: true];
            let _: () = msg_send![&*root, addSubview: &*view];
            // UITextInteractionModeEditable: UIKit's own cursor, handles, loupe
            // and edit menu over zgx's text.
            let interaction: Retained<AnyObject> =
                msg_send![class!(UITextInteraction), textInteractionForMode: 0isize];
            let _: () = msg_send![&*interaction, setTextInput: &*view];
            let _: () = msg_send![&*view, addInteraction: &*interaction];
            view
        };
        let _ = NSObject::class();
        Some(Self {
            view,
            state,
            focused: Cell::new(false),
            size: Cell::new((0., 0.)),
        })
    }
    /// The focused editor changed: the keyboard comes or goes with it.
    /// Whether the keyboard is still this view's: a rotation hides it for a
    /// moment and it comes back, a drag that dismissed it took this away.
    pub(crate) fn holds_keyboard(&self) -> bool {
        // SAFETY: UIResponder's getter, on the main thread.
        unsafe { msg_send![&*self.view, isFirstResponder] }
    }
    pub(crate) fn focus(&self, editor: Option<EditorHandle>) {
        let wanted = editor.is_some();
        if let Some(editor) = &editor {
            editor.set_platform_selection(true);
        }
        self.state.editing.set(wanted);
        self.state.store.borrow_mut().focus(editor);
        self.state.told_len.set(self.state.store.borrow().len());
        if wanted != self.focused.replace(wanted) {
            // SAFETY: UIResponder's methods, on the main thread.
            unsafe {
                if wanted {
                    // Above what came since: native views' planes and hosts.
                    let parent: Option<Retained<AnyObject>> = msg_send![&*self.view, superview];
                    if let Some(parent) = parent {
                        let _: () = msg_send![&*parent, bringSubviewToFront: &*self.view];
                    }
                    let _: Bool = msg_send![&*self.view, becomeFirstResponder];
                } else {
                    let _: Bool = msg_send![&*self.view, resignFirstResponder];
                }
            }
        }
    }
    /// Keep the view over the focused editor (`bounds`, in the window's points).
    pub(crate) fn place(&self, bounds: zgx::scene::Rect) {
        if self.state.origin.get() == (bounds.x, bounds.y)
            && self.size.get() == (bounds.width, bounds.height)
        {
            return;
        }
        self.state.origin.set((bounds.x, bounds.y));
        self.size.set((bounds.width, bounds.height));
        let frame = CGRect {
            origin: CGPoint {
                x: bounds.x as f64,
                y: bounds.y as f64,
            },
            size: CGSize {
                width: bounds.width as f64,
                height: bounds.height as f64,
            },
        };
        // SAFETY: UIView's setter, on the main thread.
        unsafe {
            let _: () = msg_send![&*self.view, setFrame: frame];
        }
    }
    /// Return was pressed since the last call.
    pub(crate) fn take_enter(&self) -> bool {
        self.state.enter.take()
    }
    /// After an event: tell UIKit what changed that it did not do itself, or
    /// the keyboard suggests from text that is gone.
    pub(crate) fn after_dispatch(&self) {
        let changes = {
            let mut store = self.state.store.borrow_mut();
            let told = self.state.told_len.get();
            let changes = store.changes(told);
            if !changes.is_empty() {
                self.state.told_len.set(store.len());
            }
            changes
        };
        for change in changes {
            match change {
                Change::CompositionCancelled => {
                    self.state.store.borrow_mut().end_composition();
                    tell(
                        &self.state,
                        &self.view,
                        sel!(textWillChange:),
                        sel!(textDidChange:),
                        || {},
                    );
                }
                Change::Text { .. } => tell(
                    &self.state,
                    &self.view,
                    sel!(textWillChange:),
                    sel!(textDidChange:),
                    || {},
                ),
                Change::Selection => tell(
                    &self.state,
                    &self.view,
                    sel!(selectionWillChange:),
                    sel!(selectionDidChange:),
                    || {},
                ),
            }
        }
    }
}
