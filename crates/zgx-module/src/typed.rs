//! Arguments and results by type: what [`export`](crate::export) reads a
//! function's arguments with and makes its result of.
//!
//! Reading an argument makes the tests of kind a function written by hand
//! makes and no more: a `&str` is the runtime's text, a [`List`]'s items and
//! a record's borrowed fields are read where they are. `String`, `Vec` and
//! the maps copy, by the author's choice of type.

use std::{
    collections::{BTreeMap, HashMap},
    marker::PhantomData,
};

use crate::{Ref, Text, Value};

/// A type a function takes as an argument, read from the value a script
/// passed. `#[derive(Record)]` writes it for a struct, read from a map by
/// its fields' names.
pub trait FromValue<'a>: Sized {
    /// What a script must pass, as an error says it: `"a string"`.
    const KIND: &'static str;
    /// The value as this type; none if it is another kind.
    fn read(value: Ref<'a>) -> Option<Self>;
    /// Whether [`read`](Self::read) would give one: the same tests, with
    /// nothing made.
    #[inline]
    fn is(value: Ref<'a>) -> bool {
        Self::read(value).is_some()
    }
    /// What is wrong with a value that isn't one, as the end of "argument 1
    /// (text) …": `"must be a string, not a number"`.
    #[cold]
    fn wrong(value: Ref<'a>) -> String {
        format!("must be {}, not {}", Self::KIND, value.kind())
    }
}

/// A type a function returns, made into the value a script is given.
/// `#[derive(Record)]` writes it for a struct, as a map of its fields.
pub trait IntoValue {
    fn into_value(self) -> Value;
}

impl<'a> FromValue<'a> for Ref<'a> {
    const KIND: &'static str = "a value";
    #[inline(always)]
    fn read(value: Ref<'a>) -> Option<Self> {
        Some(value)
    }
}
impl<'a> FromValue<'a> for Value {
    const KIND: &'static str = "a value";
    #[inline]
    fn read(value: Ref<'a>) -> Option<Self> {
        Some(value.to_owned())
    }
    #[inline(always)]
    fn is(_: Ref<'a>) -> bool {
        true
    }
}
impl<'a> FromValue<'a> for bool {
    const KIND: &'static str = "a boolean";
    #[inline(always)]
    fn read(value: Ref<'a>) -> Option<Self> {
        value.bool()
    }
}
impl<'a> FromValue<'a> for f64 {
    const KIND: &'static str = "a number";
    #[inline(always)]
    fn read(value: Ref<'a>) -> Option<Self> {
        value.num()
    }
}
impl<'a> FromValue<'a> for f32 {
    const KIND: &'static str = "a number";
    #[inline(always)]
    fn read(value: Ref<'a>) -> Option<Self> {
        value.num().map(|n| n as f32)
    }
}
macro_rules! whole {
    ($($int:ty),*) => {$(
        impl<'a> FromValue<'a> for $int {
            const KIND: &'static str = "a whole number";
            #[inline]
            fn read(value: Ref<'a>) -> Option<Self> {
                let n = value.num()?;
                // (As a script's numbers are: whole, and one this type
                // holds exactly.)
                let whole = n as $int;
                (whole as f64 == n).then_some(whole)
            }
            #[cold]
            fn wrong(value: Ref<'a>) -> String {
                match value.num() {
                    Some(n) => format!(
                        "must be a whole number from {} to {}, not {n}",
                        <$int>::MIN,
                        <$int>::MAX
                    ),
                    None => format!("must be a whole number, not {}", value.kind()),
                }
            }
        }
        impl IntoValue for $int {
            #[inline]
            fn into_value(self) -> Value {
                Value::from(self as f64)
            }
        }
    )*};
}
whole!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl<'a> FromValue<'a> for &'a str {
    const KIND: &'static str = "a string";
    #[inline(always)]
    fn read(value: Ref<'a>) -> Option<Self> {
        value.str()
    }
}
/// A string of any length: one the runtime holds in pieces (past 64 KiB)
/// too, which `&str` doesn't read.
impl<'a> FromValue<'a> for Text<'a> {
    const KIND: &'static str = "a string";
    #[inline]
    fn read(value: Ref<'a>) -> Option<Self> {
        value.text()
    }
}
impl<'a> FromValue<'a> for String {
    const KIND: &'static str = "a string";
    #[inline]
    fn read(value: Ref<'a>) -> Option<Self> {
        value.text().map(|text| String::from(&*text))
    }
    #[inline]
    fn is(value: Ref<'a>) -> bool {
        value.is_string()
    }
}
/// `nil`, or the argument not passed: none.
impl<'a, T: FromValue<'a>> FromValue<'a> for Option<T> {
    const KIND: &'static str = T::KIND;
    #[inline]
    fn read(value: Ref<'a>) -> Option<Self> {
        if value.is_nil() {
            Some(None)
        } else {
            T::read(value).map(Some)
        }
    }
    #[inline]
    fn is(value: Ref<'a>) -> bool {
        value.is_nil() || T::is(value)
    }
    #[cold]
    fn wrong(value: Ref<'a>) -> String {
        T::wrong(value).replacen(", not ", " or nil, not ", 1)
    }
}

/// What is wrong with a list one of whose items isn't a `T`, or that isn't
/// a list.
#[cold]
fn wrong_list<'a, T: FromValue<'a>>(value: Ref<'a>) -> String {
    match value.items().enumerate().find(|(_, item)| !T::is(*item)) {
        Some((at, item)) => format!("item {} {}", at + 1, T::wrong(item)),
        None => format!("must be a list, not {}", value.kind()),
    }
}
/// What is wrong with a map one of whose values isn't a `T`, or that isn't
/// a map.
#[cold]
fn wrong_map<'a, T: FromValue<'a>>(value: Ref<'a>) -> String {
    match value.fields().find(|(_, field)| !T::is(*field)) {
        Some((key, field)) => format!("field `{key}` {}", T::wrong(field)),
        None => format!("must be a map, not {}", value.kind()),
    }
}

/// A list argument, lent: its items read where the runtime holds them, as
/// `T`s. Each was seen to be one as the function was called.
pub struct List<'a, T> {
    of: Ref<'a>,
    _items: PhantomData<T>,
}
impl<'a, T: FromValue<'a>> List<'a, T> {
    /// How many items it has.
    #[inline]
    pub fn len(&self) -> usize {
        self.of.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Its item at `index`, from 0.
    #[inline]
    pub fn get(&self, index: usize) -> Option<T> {
        self.of.item(index).and_then(T::read)
    }
    /// Its items in order.
    #[inline]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = T> + DoubleEndedIterator + 'a
    where
        T: 'a,
    {
        // (Each is a `T`: `read` saw so.)
        self.of
            .items()
            .map(|item| T::read(item).expect("a list's item, seen to be of its type"))
    }
    /// The list itself, as the runtime's value.
    pub fn value(&self) -> Ref<'a> {
        self.of
    }
}
impl<'a, T: FromValue<'a>> FromValue<'a> for List<'a, T> {
    const KIND: &'static str = "a list";
    #[inline]
    fn read(value: Ref<'a>) -> Option<Self> {
        (value.is_list() && value.items().all(T::is)).then_some(List {
            of: value,
            _items: PhantomData,
        })
    }
    #[cold]
    fn wrong(value: Ref<'a>) -> String {
        wrong_list::<T>(value)
    }
}
impl<'a, T: FromValue<'a>> FromValue<'a> for Vec<T> {
    const KIND: &'static str = "a list";
    #[inline]
    fn read(value: Ref<'a>) -> Option<Self> {
        if !value.is_list() {
            return None;
        }
        value.items().map(T::read).collect()
    }
    #[inline]
    fn is(value: Ref<'a>) -> bool {
        value.is_list() && value.items().all(T::is)
    }
    #[cold]
    fn wrong(value: Ref<'a>) -> String {
        wrong_list::<T>(value)
    }
}

/// A map argument, lent: its fields read where the runtime holds them, in
/// key order, their values as `T`s. Each was seen to be one as the function
/// was called.
pub struct Map<'a, T> {
    of: Ref<'a>,
    _values: PhantomData<T>,
}
impl<'a, T: FromValue<'a>> Map<'a, T> {
    /// How many fields it has.
    #[inline]
    pub fn len(&self) -> usize {
        self.of.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The value of its field `key`.
    #[inline]
    pub fn get(&self, key: &str) -> Option<T> {
        self.of.field(key).and_then(T::read)
    }
    /// Its fields in key order.
    #[inline]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&'a str, T)> + 'a
    where
        T: 'a,
    {
        self.of.fields().map(|(key, field)| {
            (
                key,
                T::read(field).expect("a map's value, seen to be of its type"),
            )
        })
    }
    /// The map itself, as the runtime's value.
    pub fn value(&self) -> Ref<'a> {
        self.of
    }
}
impl<'a, T: FromValue<'a>> FromValue<'a> for Map<'a, T> {
    const KIND: &'static str = "a map";
    #[inline]
    fn read(value: Ref<'a>) -> Option<Self> {
        (value.is_map() && value.fields().all(|(_, field)| T::is(field))).then_some(Map {
            of: value,
            _values: PhantomData,
        })
    }
    #[cold]
    fn wrong(value: Ref<'a>) -> String {
        wrong_map::<T>(value)
    }
}
macro_rules! maps {
    ($($map:ident),*) => {$(
        impl<'a, T: FromValue<'a>> FromValue<'a> for $map<String, T> {
            const KIND: &'static str = "a map";
            #[inline]
            fn read(value: Ref<'a>) -> Option<Self> {
                if !value.is_map() {
                    return None;
                }
                value
                    .fields()
                    .map(|(key, field)| Some((key.to_owned(), T::read(field)?)))
                    .collect()
            }
            #[inline]
            fn is(value: Ref<'a>) -> bool {
                value.is_map() && value.fields().all(|(_, field)| T::is(field))
            }
            #[cold]
            fn wrong(value: Ref<'a>) -> String {
                wrong_map::<T>(value)
            }
        }
        impl<T: IntoValue> IntoValue for $map<String, T> {
            fn into_value(self) -> Value {
                Value::record(self.into_iter().map(|(key, value)| (key, value.into_value())))
            }
        }
    )*};
}
maps!(HashMap, BTreeMap);

impl<T> IntoValue for List<'_, T> {
    #[inline]
    fn into_value(self) -> Value {
        self.of.to_owned()
    }
}
impl<T> IntoValue for Map<'_, T> {
    #[inline]
    fn into_value(self) -> Value {
        self.of.to_owned()
    }
}

impl IntoValue for Value {
    #[inline(always)]
    fn into_value(self) -> Value {
        self
    }
}
impl IntoValue for Ref<'_> {
    #[inline]
    fn into_value(self) -> Value {
        self.to_owned()
    }
}
impl IntoValue for () {
    #[inline(always)]
    fn into_value(self) -> Value {
        Value::NIL
    }
}
impl IntoValue for bool {
    #[inline(always)]
    fn into_value(self) -> Value {
        Value::from(self)
    }
}
impl IntoValue for f64 {
    #[inline(always)]
    fn into_value(self) -> Value {
        Value::from(self)
    }
}
impl IntoValue for f32 {
    #[inline(always)]
    fn into_value(self) -> Value {
        Value::from(f64::from(self))
    }
}
impl IntoValue for &str {
    #[inline]
    fn into_value(self) -> Value {
        Value::from(self)
    }
}
impl IntoValue for String {
    #[inline]
    fn into_value(self) -> Value {
        Value::from(self.as_str())
    }
}
impl IntoValue for Text<'_> {
    #[inline]
    fn into_value(self) -> Value {
        match self {
            Text::Lent(text) => Value::from(text),
            Text::Held(value) => value,
        }
    }
}
impl<T: IntoValue> IntoValue for Option<T> {
    #[inline]
    fn into_value(self) -> Value {
        self.map_or(Value::NIL, T::into_value)
    }
}
impl<T: IntoValue> IntoValue for Vec<T> {
    fn into_value(self) -> Value {
        Value::list(self.into_iter().map(T::into_value))
    }
}
impl<T: IntoValue + Clone> IntoValue for &[T] {
    fn into_value(self) -> Value {
        Value::list(self.iter().cloned().map(T::into_value))
    }
}
