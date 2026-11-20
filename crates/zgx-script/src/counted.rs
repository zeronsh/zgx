//! What a [`Value`](crate::Value) holds its strings, lists and maps in, laid
//! out as zgx says and not as the compiler chooses: native modules read
//! values where they are (`modules.rs`, `zgx_module::abi`), and what they
//! read has to stay where it is from one compiler to the next.
//!
//! [`Slice`] is a counted slice of zgx's own, as `Rc<[T]>` is but with its
//! counts and items at stated places: a list's ([`List`]) and a map's
//! ([`Map`], of [`Field`]s). [`Str`] is a string std's own `Rc<str>` holds,
//! kept as a pointer to its text and a length, made and let go of through
//! `Rc`'s documented raw functions alone: turning one into an `Rc<str>` and
//! back copies nothing.

use std::{
    alloc::{self, Layout},
    borrow::Borrow,
    cell::Cell,
    fmt,
    hash::{Hash, Hasher},
    marker::PhantomData,
    ops::Deref,
    ptr::NonNull,
    rc::Rc,
};

use crate::Value;

/// A string a value holds: text shared, not copied, by each clone, as an
/// `Rc<str>` is, and one's worth of memory: `Str::from(rc)` and
/// `Rc::<str>::from(str)` change which type holds it and nothing else.
// A pointer to the text and its length, in that order: what a module reads.
#[repr(C)]
pub struct Str {
    text: NonNull<u8>,
    len: usize,
    _held: PhantomData<Rc<str>>,
}

impl Str {
    #[inline]
    fn raw(&self) -> *const str {
        std::ptr::slice_from_raw_parts(self.text.as_ptr().cast_const(), self.len) as *const str
    }
    /// The same text as an `Rc<str>`: held once more, not copied.
    #[inline]
    pub fn rc(&self) -> Rc<str> {
        // SAFETY: `raw` is an `Rc<str>`'s `into_raw`, held by `self`.
        unsafe {
            Rc::increment_strong_count(self.raw());
            Rc::from_raw(self.raw())
        }
    }
    /// Whether both are the same text in memory.
    #[inline]
    pub fn ptr_eq(a: &Str, b: &Str) -> bool {
        a.text == b.text
    }
    /// Where its text is.
    #[inline]
    pub fn as_ptr(this: &Str) -> *const u8 {
        this.text.as_ptr()
    }
    /// How many hold this text.
    pub fn strong_count(this: &Str) -> usize {
        let rc = std::mem::ManuallyDrop::new(
            // SAFETY: As `rc`, and not dropped: `self` still holds it.
            unsafe { Rc::from_raw(this.raw()) },
        );
        Rc::strong_count(&rc)
    }
}
impl From<Rc<str>> for Str {
    #[inline]
    fn from(text: Rc<str>) -> Self {
        let len = text.len();
        // SAFETY: `into_raw` is never null.
        let text = unsafe { NonNull::new_unchecked(Rc::into_raw(text) as *mut u8) };
        Str {
            text,
            len,
            _held: PhantomData,
        }
    }
}
impl From<&Rc<str>> for Str {
    #[inline]
    fn from(text: &Rc<str>) -> Self {
        Str::from(text.clone())
    }
}
impl From<Str> for Rc<str> {
    #[inline]
    fn from(text: Str) -> Self {
        let text = std::mem::ManuallyDrop::new(text);
        // SAFETY: An `Rc<str>`'s `into_raw`, handed back once.
        unsafe { Rc::from_raw(text.raw()) }
    }
}
impl From<&Str> for Rc<str> {
    #[inline]
    fn from(text: &Str) -> Self {
        text.rc()
    }
}
impl From<&str> for Str {
    #[inline]
    fn from(text: &str) -> Self {
        Str::from(Rc::<str>::from(text))
    }
}
impl From<String> for Str {
    #[inline]
    fn from(text: String) -> Self {
        Str::from(Rc::<str>::from(text))
    }
}
impl From<&String> for Str {
    #[inline]
    fn from(text: &String) -> Self {
        Str::from(text.as_str())
    }
}
impl From<Box<str>> for Str {
    fn from(text: Box<str>) -> Self {
        Str::from(Rc::<str>::from(text))
    }
}
impl From<std::borrow::Cow<'_, str>> for Str {
    fn from(text: std::borrow::Cow<'_, str>) -> Self {
        Str::from(Rc::<str>::from(text))
    }
}
impl Default for Str {
    fn default() -> Self {
        Str::from("")
    }
}
impl Clone for Str {
    #[inline]
    fn clone(&self) -> Self {
        // SAFETY: An `Rc<str>`'s `into_raw`, held by `self`.
        unsafe { Rc::increment_strong_count(self.raw()) };
        Str {
            text: self.text,
            len: self.len,
            _held: PhantomData,
        }
    }
}
impl Drop for Str {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: An `Rc<str>`'s `into_raw`, given back once.
        drop(unsafe { Rc::from_raw(self.raw()) });
    }
}
impl Deref for Str {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        // SAFETY: The text an `Rc<str>` holds for as long as `self` lives.
        unsafe { &*self.raw() }
    }
}
impl AsRef<str> for Str {
    fn as_ref(&self) -> &str {
        self
    }
}
impl Borrow<str> for Str {
    fn borrow(&self) -> &str {
        self
    }
}
impl fmt::Debug for Str {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}
impl fmt::Display for Str {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}
impl PartialEq for Str {
    #[inline]
    fn eq(&self, other: &Str) -> bool {
        // (The same text in memory is equal without being read, as an
        // `Rc<str>`'s is.)
        (self.text == other.text && self.len == other.len) || **self == **other
    }
}
impl Eq for Str {}
impl PartialEq<str> for Str {
    fn eq(&self, other: &str) -> bool {
        **self == *other
    }
}
impl PartialEq<&str> for Str {
    fn eq(&self, other: &&str) -> bool {
        **self == **other
    }
}
impl PartialEq<Str> for str {
    fn eq(&self, other: &Str) -> bool {
        *self == **other
    }
}
impl PartialEq<Str> for &str {
    fn eq(&self, other: &Str) -> bool {
        **self == **other
    }
}
impl PartialEq<Rc<str>> for Str {
    fn eq(&self, other: &Rc<str>) -> bool {
        **self == **other
    }
}
impl PartialEq<Str> for Rc<str> {
    fn eq(&self, other: &Str) -> bool {
        **self == **other
    }
}
impl PartialEq<String> for Str {
    fn eq(&self, other: &String) -> bool {
        **self == **other
    }
}
impl PartialOrd for Str {
    fn partial_cmp(&self, other: &Str) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Str {
    fn cmp(&self, other: &Str) -> std::cmp::Ordering {
        (**self).cmp(&**other)
    }
}
impl Hash for Str {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

/// A counted slice's counts: how many hold it, and how many more would if
/// it were still there (its [`WeakSlice`]s, and one for all who hold it).
/// Its items follow, [`ITEMS_AT`] bytes from its start: a slice points at
/// its items, its counts that far before them, as an `Rc<str>`'s text has
/// its own, so holding and letting go of a string, a list and a map are the
/// same few instructions.
#[repr(C)]
struct Counts {
    strong: Cell<usize>,
    weak: Cell<usize>,
}
/// Where a slice's items begin, from its counts.
const ITEMS_AT: usize = size_of::<Counts>();

/// A slice shared, not copied, by each clone: zgx's own `Rc<[T]>`, its
/// layout stated. A [`List`] and a [`Map`] are one.
// A pointer to its items (its counts `ITEMS_AT` before them) and how many
// items, in that order: what a module reads.
#[repr(C)]
pub struct Slice<T> {
    at: NonNull<T>,
    len: usize,
    _held: PhantomData<Rc<[T]>>,
}
/// A list's items.
pub type List = Slice<Value>;
/// A map's fields, sorted by key.
pub type Map = Slice<Field>;

/// A map's field: its key and its value. (`field.0` and `field.1`, as the
/// pair it was.)
#[repr(C)]
#[derive(Clone, Debug, PartialEq)]
pub struct Field(pub Str, pub Value);
impl<K: Into<Str>> From<(K, Value)> for Field {
    #[inline]
    fn from((key, value): (K, Value)) -> Self {
        Field(key.into(), value)
    }
}

impl<T> Slice<T> {
    /// Items start a whole number of them past the counts.
    const ALIGNED: () = assert!(align_of::<T>() <= align_of::<Counts>());

    fn layout(len: usize) -> Layout {
        let () = Self::ALIGNED;
        let items = Layout::array::<T>(len).expect("a slice's size");
        Layout::new::<Counts>()
            .extend(items)
            .expect("a slice's size")
            .0
            .pad_to_align()
    }
    /// Room for `len` items, held once, its items unwritten: where they go.
    fn room(len: usize) -> NonNull<T> {
        let layout = Self::layout(len);
        // SAFETY: The layout is never empty: it has its counts.
        let Some(counts) = NonNull::new(unsafe { alloc::alloc(layout) }.cast::<Counts>()) else {
            alloc::handle_alloc_error(layout)
        };
        // SAFETY: Room for the counts, just made, and the items after them.
        unsafe {
            counts.as_ptr().write(Counts {
                strong: Cell::new(1),
                weak: Cell::new(1),
            });
            NonNull::new_unchecked(counts.as_ptr().cast::<u8>().add(ITEMS_AT).cast())
        }
    }
    /// The counts of the slice whose items are at `at`.
    #[inline(always)]
    fn counts_of<'a>(at: NonNull<T>) -> &'a Counts {
        // SAFETY: A slice's counts are `ITEMS_AT` before its items, alive
        // while anything, strong or weak, holds it.
        unsafe { &*at.as_ptr().cast::<u8>().sub(ITEMS_AT).cast::<Counts>() }
    }
    #[inline(always)]
    fn counts(&self) -> &Counts {
        Self::counts_of(self.at)
    }
    /// Frees the memory of the slice of `len` items at `at`.
    ///
    /// # Safety
    /// Nothing holds it, strongly or weakly, and its items were dropped.
    unsafe fn free(at: NonNull<T>, len: usize) {
        // SAFETY: The allocation `room` made for `len` items, whose layout
        // was checked as it was made: the same size and alignment here,
        // with nothing left to check.
        unsafe {
            let align = align_of::<Counts>();
            let size = (ITEMS_AT + len * size_of::<T>() + align - 1) & !(align - 1);
            let layout = Layout::from_size_align_unchecked(size, align);
            debug_assert!(layout == Self::layout(len));
            alloc::dealloc(at.as_ptr().cast::<u8>().sub(ITEMS_AT), layout);
        }
    }
    #[inline(always)]
    fn held(at: NonNull<T>, len: usize) -> Self {
        Slice {
            at,
            len,
            _held: PhantomData,
        }
    }
    /// A slice of what `items` gives, which must give `len` of them: one
    /// allocation, each item written where it stays.
    pub(crate) fn collected(len: usize, items: impl Iterator<Item = T>) -> Self {
        /// What was written so far, dropped if making the rest panics.
        struct Partly<T> {
            at: NonNull<T>,
            written: usize,
            len: usize,
        }
        impl<T> Drop for Partly<T> {
            fn drop(&mut self) {
                // SAFETY: `written` items are there, in room for `len`.
                unsafe {
                    std::ptr::slice_from_raw_parts_mut(self.at.as_ptr(), self.written)
                        .drop_in_place();
                    Slice::<T>::free(self.at, self.len);
                }
            }
        }
        let mut partly = Partly {
            at: Self::room(len),
            written: 0,
            len,
        };
        for item in items {
            assert!(
                partly.written < len,
                "an iterator gave more items than it said"
            );
            // SAFETY: Under `len`, so inside the allocation, and unwritten.
            unsafe { partly.at.as_ptr().add(partly.written).write(item) };
            partly.written += 1;
        }
        assert_eq!(
            partly.written, len,
            "an iterator gave fewer items than it said"
        );
        let at = partly.at;
        std::mem::forget(partly);
        Self::held(at, len)
    }
    /// A slice of the `len` items at `items`, which move into it as one
    /// copy.
    ///
    /// # Safety
    /// `len` items are at `items`, and the caller no longer owns them.
    #[inline]
    unsafe fn moved(items: *const T, len: usize) -> Self {
        let at = Self::room(len);
        // SAFETY: Room for `len` items, apart from where they were.
        unsafe { std::ptr::copy_nonoverlapping(items, at.as_ptr(), len) };
        Self::held(at, len)
    }
    /// A slice of `len` items, the `i`th what `item(i)` gives, written
    /// where it stays with nothing checked between them. (An `item` that
    /// panics leaves what was written unfreed, so it must not.)
    #[inline]
    pub(crate) fn filled(len: usize, mut item: impl FnMut(usize) -> T) -> Self {
        let at = Self::room(len);
        for i in 0..len {
            // SAFETY: Under `len`, so inside the allocation, and unwritten.
            unsafe { at.as_ptr().add(i).write(item(i)) };
        }
        Self::held(at, len)
    }
    /// A slice of `len` items written by `write`, which is handed the room
    /// for them and must write every one: for a caller that fills them from
    /// several places, as it would a vector's spare room. (A `write` that
    /// panics leaves what it wrote unfreed.)
    #[inline]
    pub(crate) fn written(
        len: usize,
        write: impl FnOnce(&mut [std::mem::MaybeUninit<T>]) -> usize,
    ) -> Self {
        let at = Self::room(len);
        // SAFETY: Room for `len` items, unwritten, and nothing else's yet.
        let wrote = write(unsafe { std::slice::from_raw_parts_mut(at.as_ptr().cast(), len) });
        assert_eq!(wrote, len, "every item written");
        Self::held(at, len)
    }
    /// A slice of `items` from `from` on, which leave the vector as one
    /// copy: what gathering a table's items at the end of a shared vector
    /// makes of them, with no walk over them.
    #[inline]
    pub(crate) fn taken(items: &mut Vec<T>, from: usize) -> Self {
        let len = items.len() - from;
        // SAFETY: The vector's last `len` items move, and it then holds
        // only those before them.
        unsafe {
            let moved = Self::moved(items.as_ptr().add(from), len);
            items.set_len(from);
            moved
        }
    }
    /// Whether both are the same slice in memory.
    #[inline]
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        a.at == b.at
    }
    /// Where its items are.
    #[inline]
    pub fn as_ptr(this: &Self) -> *const T {
        this.at.as_ptr()
    }
    /// How many hold it.
    #[inline]
    pub fn strong_count(this: &Self) -> usize {
        this.counts().strong.get()
    }
    /// How many would hold it if it were still there.
    #[inline]
    pub fn weak_count(this: &Self) -> usize {
        this.counts().weak.get() - 1
    }
    /// A hold that keeps nothing alive: [`WeakSlice::upgrade`] gives the
    /// slice back while something else still holds it.
    #[inline]
    pub fn downgrade(this: &Self) -> WeakSlice<T> {
        let weak = &this.counts().weak;
        weak.set(weak.get() + 1);
        WeakSlice {
            at: this.at,
            len: this.len,
            _held: PhantomData,
        }
    }
    /// Its items to change, if nothing else holds it.
    #[inline]
    pub fn get_mut(this: &mut Self) -> Option<&mut [T]> {
        let counts = this.counts();
        // SAFETY: Held by `this` alone, so nothing else reads the items.
        (counts.strong.get() == 1 && counts.weak.get() == 1)
            .then(|| unsafe { std::slice::from_raw_parts_mut(this.at.as_ptr(), this.len) })
    }
    /// The last hold gone: its items dropped, and its memory freed unless a
    /// weak hold still looks at its counts.
    #[cold]
    #[inline(never)]
    fn drop_slow(&mut self) {
        // SAFETY: Nothing holds the items any more; the counts stay until
        // the last weak hold goes, the strong holds' own among them.
        unsafe {
            std::ptr::slice_from_raw_parts_mut(self.at.as_ptr(), self.len).drop_in_place();
            let weak = self.counts().weak.get() - 1;
            self.counts().weak.set(weak);
            if weak == 0 {
                Self::free(self.at, self.len);
            }
        }
    }
}
impl<T> Clone for Slice<T> {
    #[inline]
    fn clone(&self) -> Self {
        // (As `Rc`'s: one more, and a count that came back round to none
        // stops the program.)
        let strong = &self.counts().strong;
        let held = strong.get().wrapping_add(1);
        strong.set(held);
        if held == 0 {
            std::process::abort();
        }
        Self::held(self.at, self.len)
    }
}
impl<T> Drop for Slice<T> {
    #[inline]
    fn drop(&mut self) {
        let strong = &self.counts().strong;
        let held = strong.get() - 1;
        strong.set(held);
        if held == 0 {
            self.drop_slow();
        }
    }
}
impl<T> Deref for Slice<T> {
    type Target = [T];
    #[inline]
    fn deref(&self) -> &[T] {
        // SAFETY: `len` items, alive as long as `self`.
        unsafe { std::slice::from_raw_parts(self.at.as_ptr(), self.len) }
    }
}
impl<T> AsRef<[T]> for Slice<T> {
    fn as_ref(&self) -> &[T] {
        self
    }
}
impl<T> Borrow<[T]> for Slice<T> {
    fn borrow(&self) -> &[T] {
        self
    }
}
impl<T> Default for Slice<T> {
    fn default() -> Self {
        Self::held(Self::room(0), 0)
    }
}
impl<T: fmt::Debug> fmt::Debug for Slice<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}
impl<T: PartialEq> PartialEq for Slice<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
impl<T> From<Vec<T>> for Slice<T> {
    #[inline]
    fn from(mut items: Vec<T>) -> Self {
        // SAFETY: The vector's items move, and it then holds none: it frees
        // its memory alone.
        unsafe {
            let moved = Self::moved(items.as_ptr(), items.len());
            items.set_len(0);
            moved
        }
    }
}
impl<T, const N: usize> From<[T; N]> for Slice<T> {
    #[inline]
    fn from(items: [T; N]) -> Self {
        let items = std::mem::ManuallyDrop::new(items);
        // SAFETY: The array's items move, and it is not dropped.
        unsafe { Self::moved(items.as_ptr(), N) }
    }
}
impl<T: Clone> From<&[T]> for Slice<T> {
    #[inline]
    fn from(items: &[T]) -> Self {
        // SAFETY: `i` is under the slice's length.
        Self::filled(items.len(), |i| unsafe { items.get_unchecked(i) }.clone())
    }
}
impl<T: Clone> From<&Vec<T>> for Slice<T> {
    fn from(items: &Vec<T>) -> Self {
        Self::from(items.as_slice())
    }
}
impl<T: Clone> From<Rc<[T]>> for Slice<T> {
    /// (A copy: the items cloned into a slice of zgx's own.)
    fn from(items: Rc<[T]>) -> Self {
        Self::from(&*items)
    }
}
impl<T> FromIterator<T> for Slice<T> {
    /// One allocation where the iterator says how many it gives (a slice's,
    /// a range's, a `Vec`'s, mapped or not); else gathered first.
    #[inline]
    fn from_iter<I: IntoIterator<Item = T>>(items: I) -> Self {
        let items = items.into_iter();
        match items.size_hint() {
            (least, Some(most)) if least == most => Self::collected(least, items),
            _ => Self::from(items.collect::<Vec<T>>()),
        }
    }
}
// (Of `(Str, Value)` and no other pair, so `("key".into(), value)` says
// which key it means, as it did when a field was that pair.)
impl FromIterator<(Str, Value)> for Slice<Field> {
    #[inline]
    fn from_iter<I: IntoIterator<Item = (Str, Value)>>(fields: I) -> Self {
        fields.into_iter().map(Field::from).collect()
    }
}
impl From<Vec<(Str, Value)>> for Slice<Field> {
    #[inline]
    fn from(fields: Vec<(Str, Value)>) -> Self {
        fields.into_iter().collect()
    }
}
impl<const N: usize> From<[(Str, Value); N]> for Slice<Field> {
    #[inline]
    fn from(fields: [(Str, Value); N]) -> Self {
        Slice::from(fields.map(|(key, value)| Field(key, value)))
    }
}
impl<'a, T> IntoIterator for &'a Slice<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// A hold on a [`Slice`] that keeps nothing alive ([`Slice::downgrade`]).
pub struct WeakSlice<T> {
    at: NonNull<T>,
    len: usize,
    _held: PhantomData<Rc<[T]>>,
}
impl<T> WeakSlice<T> {
    #[inline]
    fn counts(&self) -> &Counts {
        Slice::counts_of(self.at)
    }
    /// The slice, while something still holds it.
    #[inline]
    pub fn upgrade(&self) -> Option<Slice<T>> {
        let strong = &self.counts().strong;
        if strong.get() == 0 {
            return None;
        }
        strong.set(strong.get() + 1);
        Some(Slice::held(self.at, self.len))
    }
    /// How many hold the slice: 0 once it is gone.
    pub fn strong_count(&self) -> usize {
        self.counts().strong.get()
    }
    /// Whether both look at the same slice.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        self.at == other.at
    }
}
impl<T> Clone for WeakSlice<T> {
    fn clone(&self) -> Self {
        let weak = &self.counts().weak;
        weak.set(weak.get() + 1);
        WeakSlice {
            at: self.at,
            len: self.len,
            _held: PhantomData,
        }
    }
}
impl<T> Drop for WeakSlice<T> {
    fn drop(&mut self) {
        let weak = self.counts().weak.get() - 1;
        self.counts().weak.set(weak);
        if weak == 0 {
            // SAFETY: The last hold of any kind: the items went with the
            // last strong one.
            unsafe { Slice::free(self.at, self.len) };
        }
    }
}
impl<T> fmt::Debug for WeakSlice<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(WeakSlice)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A value's string, list and map are a pointer and a length each, the
    /// field a key then a value: the layout native modules read.
    #[test]
    fn the_layout_is_the_one_stated() {
        assert_eq!(size_of::<Str>(), 16);
        assert_eq!(size_of::<List>(), 16);
        assert_eq!(
            (
                size_of::<Field>(),
                std::mem::offset_of!(Field, 0),
                std::mem::offset_of!(Field, 1)
            ),
            (40, 0, 16)
        );
        assert_eq!(ITEMS_AT, 16);
        let text = Str::from("stated");
        // SAFETY: Two words, the first its text's address.
        let words: [usize; 2] = unsafe { std::mem::transmute_copy(&text) };
        assert_eq!(words, [text.as_ptr() as usize, 6]);
        let list: Slice<u64> = Slice::from(vec![7, 8, 9]);
        // SAFETY: Two words: its items' address, and how many.
        let words: [usize; 2] = unsafe { std::mem::transmute_copy(&list) };
        assert_eq!(words, [Slice::as_ptr(&list) as usize, 3]);
        // SAFETY: Its count, `ITEMS_AT` before its items: held once.
        assert_eq!(unsafe { *((words[0] - ITEMS_AT) as *const usize) }, 1);
        let again = list.clone();
        // SAFETY: Its count: held twice.
        assert_eq!(unsafe { *((words[0] - ITEMS_AT) as *const usize) }, 2);
        drop(again);
    }

    #[test]
    fn a_string_and_an_rc_share_their_text() {
        let rc: Rc<str> = Rc::from("shared");
        let text = Str::from(rc.clone());
        assert_eq!(text.as_ptr(), rc.as_ptr());
        assert_eq!(Rc::strong_count(&rc), 2);
        let back: Rc<str> = text.rc();
        assert!(Rc::ptr_eq(&rc, &back));
        assert_eq!(Rc::strong_count(&rc), 3);
        let moved: Rc<str> = text.into();
        assert_eq!(Rc::strong_count(&rc), 3);
        drop((back, moved));
        assert_eq!(Rc::strong_count(&rc), 1);
        assert_eq!(Str::from("a"), "a");
    }

    #[test]
    fn a_slice_drops_its_items_once_and_frees_with_its_last_hold() {
        let item = Rc::new(());
        let slice: Slice<Rc<()>> = (0..5).map(|_| item.clone()).collect();
        assert_eq!((slice.len(), Rc::strong_count(&item)), (5, 6));
        let weak = Slice::downgrade(&slice);
        let again = weak.upgrade().unwrap();
        assert!(Slice::ptr_eq(&slice, &again));
        assert_eq!(
            (Slice::strong_count(&slice), Slice::weak_count(&slice)),
            (2, 1)
        );
        drop(slice);
        assert_eq!(Rc::strong_count(&item), 6);
        drop(again);
        assert_eq!(Rc::strong_count(&item), 1);
        assert!(weak.upgrade().is_none());
        drop(weak);
        // From an iterator that doesn't say how many, and empty.
        let odd: Slice<u32> = (0..10).filter(|n| n % 2 == 1).collect();
        assert_eq!(&*odd, [1, 3, 5, 7, 9]);
        assert!(Slice::<Value>::default().is_empty());
        let mut alone: Slice<u32> = Slice::from([1, 2]);
        Slice::get_mut(&mut alone).unwrap()[0] = 9;
        let held = alone.clone();
        assert!(Slice::get_mut(&mut alone).is_none());
        assert_eq!(&*held, [9, 2]);
    }
}
