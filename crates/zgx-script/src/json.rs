//! JSON read straight into script values: no tree of serde_json's in
//! between, and each object key one string however many objects name it
//! (10,000 messages' `id` share one).

use crate::value::Value;
use mlua::{AnyUserData, Lua, MetaMethod, Table};
use rustc_hash::FxHashMap;
use serde::de::{DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use std::rc::Rc;

/// `text`, which must be JSON whole, as a value: objects as maps (their keys
/// sorted, the last of a key given twice kept, as serde_json's map keeps
/// it), arrays as lists, numbers as `f64`.
pub(crate) fn parse(text: &str) -> serde_json::Result<Value> {
    parse_bytes(text.as_bytes())
}
/// [`parse`] of bytes, which must be UTF-8 where JSON needs it.
pub(crate) fn parse_bytes(text: &[u8]) -> serde_json::Result<Value> {
    let mut keys = Keys::default();
    let mut reader = serde_json::Deserializer::from_slice(text);
    let value = Json(&mut keys).deserialize(&mut reader)?;
    reader.end()?;
    Ok(value)
}

/// The keys read so far, each made once, and the items of the lists and
/// fields of the maps being read, on one stack each: a list or map, once
/// read, is its items moved off the stack into one allocation, where a
/// vector of its own would grow (serde_json can't say how long it is) and
/// then be copied.
#[derive(Default)]
struct Keys {
    known: FxHashMap<Box<str>, Rc<str>>,
    items: Vec<Value>,
    fields: Vec<(Rc<str>, Value)>,
}
impl Keys {
    fn get(&mut self, key: &str) -> Rc<str> {
        if let Some(known) = self.known.get(key) {
            return known.clone();
        }
        let made: Rc<str> = key.into();
        self.known.insert(key.into(), made.clone());
        made
    }
}

struct Json<'a>(&'a mut Keys);
impl<'de> DeserializeSeed<'de> for Json<'_> {
    type Value = Value;
    fn deserialize<D: Deserializer<'de>>(self, reader: D) -> Result<Value, D::Error> {
        reader.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Json<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("JSON")
    }
    fn visit_bool<E>(self, b: bool) -> Result<Value, E> {
        Ok(Value::Bool(b))
    }
    fn visit_i64<E>(self, n: i64) -> Result<Value, E> {
        Ok(Value::Num(n as f64))
    }
    fn visit_u64<E>(self, n: u64) -> Result<Value, E> {
        Ok(Value::Num(n as f64))
    }
    fn visit_f64<E>(self, n: f64) -> Result<Value, E> {
        Ok(Value::Num(n))
    }
    fn visit_str<E>(self, s: &str) -> Result<Value, E> {
        Ok(Value::Str(s.into()))
    }
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Nil)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut items: A) -> Result<Value, A::Error> {
        let from = self.0.items.len();
        while let Some(item) = items.next_element_seed(Json(&mut *self.0))? {
            self.0.items.push(item);
        }
        Ok(Value::List(self.0.items.drain(from..).collect()))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Value, A::Error> {
        let from = self.0.fields.len();
        while let Some(key) = entries.next_key_seed(Key(&mut *self.0))? {
            let value = entries.next_value_seed(Json(&mut *self.0))?;
            self.0.fields.push((key, value));
        }
        let fields = &mut self.0.fields[from..];
        // Sorted, as maps are (often they come so); a key given twice keeps
        // its last value (the sort is stable, so that one comes last of its
        // kind).
        if !fields.is_sorted_by(|a, b| a.0 < b.0) {
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            if fields.windows(2).any(|w| w[0].0 == w[1].0) {
                let mut kept: Vec<(Rc<str>, Value)> = Vec::with_capacity(fields.len());
                for (key, value) in self.0.fields.drain(from..) {
                    match kept.last_mut() {
                        Some(last) if last.0 == key => last.1 = value,
                        _ => kept.push((key, value)),
                    }
                }
                return Ok(Value::Map(kept.into()));
            }
        }
        Ok(Value::Map(self.0.fields.drain(from..).collect()))
    }
}

struct Key<'a>(&'a mut Keys);
impl<'de> DeserializeSeed<'de> for Key<'_> {
    type Value = Rc<str>;
    fn deserialize<D: Deserializer<'de>>(self, reader: D) -> Result<Rc<str>, D::Error> {
        reader.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for Key<'_> {
    type Value = Rc<str>;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a key")
    }
    fn visit_str<E>(self, key: &str) -> Result<Rc<str>, E> {
        Ok(self.0.get(key))
    }
}

/// What a write to `ui.json.data`'s lists and objects says.
const READ_ONLY: &str = "ui.json.data's lists and objects are read-only: \
                         ui.json.decode(text) gives tables a script can change";

/// `ui.json.data`'s lists and objects (`JsonData` to scripts): a userdata
/// over the value, read through `__index`, `__len`, `__iter` (`for k, v in
/// data do`) and `__eq`, so data handed on whole (to `ui.state`, a list's
/// `data`) is never made into tables at all, and is its value again without
/// a walk (`value::convert`). Its parts are made as they are read and kept
/// in its user value by key, so a part read twice is the same. Not a table,
/// it fails `pairs`, `ipairs`, `next`, `rawget` and the `table` library
/// with a type error, rather than reading as empty.
pub(crate) struct Data(pub(crate) Value);

/// `ui.json.load` of a VM, resolving as its `require` does.
pub(crate) struct Load(pub mlua::Function);

/// `__iter`'s step, one per VM.
struct Step(mlua::Function);

impl mlua::UserData for Data {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field(MetaMethod::Type, "JsonData");
    }
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_function(
            MetaMethod::Index,
            |lua, (data, key): (AnyUserData, mlua::Value)| index(lua, &data, key),
        );
        methods.add_meta_method(MetaMethod::Len, |_, data, ()| {
            Ok(match &data.0 {
                Value::List(items) => items.len(),
                _ => 0,
            })
        });
        methods.add_meta_function(MetaMethod::Iter, |lua, data: AnyUserData| {
            Ok((step(lua)?, data, mlua::Value::Nil))
        });
        methods.add_meta_function(MetaMethod::Eq, |_, (a, b): (AnyUserData, AnyUserData)| {
            let (a, b) = (a.borrow::<Data>()?, b.borrow::<Data>()?);
            let id = crate::value::identity(&a.0);
            Ok(id != 0 && id == crate::value::identity(&b.0))
        });
        methods.add_meta_function(MetaMethod::NewIndex, |_, ()| {
            Err::<(), _>(mlua::Error::runtime(READ_ONLY))
        });
    }
}

/// `value` for a script to read: a list or object as `Data`, else as
/// itself.
pub(crate) fn data(lua: &Lua, value: &Value) -> mlua::Result<mlua::Value> {
    match value {
        Value::List(_) | Value::Map(_) => lua
            .create_userdata(Data(value.clone()))
            .map(mlua::Value::UserData),
        _ => crate::value::to_lua(value, lua, true),
    }
}

/// `data[key]`: an item (by its position) or a field, a list or object
/// made the first time it is read and kept.
fn index(lua: &Lua, data: &AnyUserData, key: mlua::Value) -> mlua::Result<mlua::Value> {
    let part = {
        let this = data.borrow::<Data>()?;
        let part = match (&this.0, &key) {
            (Value::List(items), mlua::Value::Integer(i)) => usize::try_from(*i)
                .ok()
                .and_then(|i| items.get(i.checked_sub(1)?)),
            (Value::List(items), mlua::Value::Number(n)) if n.fract() == 0. && *n >= 1. => {
                items.get(*n as usize - 1)
            }
            (Value::Map(_), mlua::Value::String(name)) => match name.to_str() {
                Ok(name) => this.0.field(&name),
                Err(_) => None,
            },
            _ => None,
        };
        match part {
            None => return Ok(mlua::Value::Nil),
            Some(part @ (Value::List(_) | Value::Map(_))) => part.clone(),
            Some(part) => return crate::value::to_lua(part, lua, true),
        }
    };
    let kept: Option<Table> = data.user_value()?;
    if let Some(kept) = &kept
        && let found @ (mlua::Value::UserData(_) | mlua::Value::Table(_)) =
            kept.raw_get::<mlua::Value>(&key)?
    {
        return Ok(found);
    }
    let made = data_part(lua, &part)?;
    let kept = match kept {
        Some(kept) => kept,
        None => {
            let kept = lua.create_table()?;
            data.set_user_value(&kept)?;
            kept
        }
    };
    kept.raw_set(key, &made)?;
    Ok(made)
}
/// A part as a script reads it: as a read-only table, made whole at once,
/// when it is small (a row, a paragraph's runs) or holds plain values alone
/// (a series of numbers, of any length): its items then read at a table's
/// speed, where each `Data` read is a call (60-75 ns in a release build,
/// against a table's 2-4). Lists and objects of lists and objects stay
/// `Data`, their parts made as they are read.
fn data_part(lua: &Lua, part: &Value) -> mlua::Result<mlua::Value> {
    if small(part) || plain(part) {
        return crate::value::to_lua(part, lua, true);
    }
    lua.create_userdata(Data(part.clone()))
        .map(mlua::Value::UserData)
}
/// Whether `value` holds no list or object: plain values alone.
fn plain(value: &Value) -> bool {
    match value {
        Value::List(items) => items
            .iter()
            .all(|v| !matches!(v, Value::List(_) | Value::Map(_))),
        Value::Map(fields) => fields
            .iter()
            .all(|(_, v)| !matches!(v, Value::List(_) | Value::Map(_))),
        _ => true,
    }
}
/// The most values a part made a table holds.
const TABLE_UP_TO: usize = 512;
/// Whether `value` holds `TABLE_UP_TO` values or fewer, all the way down.
fn small(value: &Value) -> bool {
    fn count(value: &Value, left: &mut usize) -> bool {
        let children: &mut dyn Iterator<Item = &Value> = match value {
            Value::List(items) => &mut items.iter(),
            Value::Map(fields) => &mut fields.iter().map(|(_, v)| v),
            _ => return true,
        };
        for child in children {
            if *left == 0 {
                return false;
            }
            *left -= 1;
            if !count(child, left) {
                return false;
            }
        }
        true
    }
    count(value, &mut TABLE_UP_TO.clone())
}

/// The VM's `__iter` step: `(data, key)` to the next key and its value,
/// past nulls (as a table holds none).
fn step(lua: &Lua) -> mlua::Result<mlua::Function> {
    if let Some(step) = lua.app_data_ref::<Step>() {
        return Ok(step.0.clone());
    }
    let step = lua.create_function(|lua, (data, key): (AnyUserData, mlua::Value)| {
        let next = {
            let this = data.borrow::<Data>()?;
            let mut at = match (&this.0, &key) {
                (_, mlua::Value::Nil) => 0,
                (Value::List(_), mlua::Value::Integer(i)) => *i as usize,
                (Value::List(_), mlua::Value::Number(n)) => *n as usize,
                (Value::Map(fields), mlua::Value::String(name)) => {
                    let name = name.to_str()?;
                    match fields.binary_search_by(|(k, _)| (**k).cmp(&*name)) {
                        Ok(i) => i + 1,
                        Err(_) => return Err(mlua::Error::runtime("invalid key to iterate")),
                    }
                }
                _ => return Err(mlua::Error::runtime("invalid key to iterate")),
            };
            match &this.0 {
                Value::List(items) => {
                    while items.get(at).is_some_and(|v| matches!(v, Value::Nil)) {
                        at += 1;
                    }
                    (at < items.len()).then(|| mlua::Value::Integer(at as i64 + 1))
                }
                Value::Map(fields) => {
                    while fields.get(at).is_some_and(|(_, v)| matches!(v, Value::Nil)) {
                        at += 1;
                    }
                    match fields.get(at) {
                        Some((name, _)) => Some(mlua::Value::String(lua.create_string(&**name)?)),
                        None => None,
                    }
                }
                _ => None,
            }
        };
        match next {
            Some(key) => {
                let value = index(lua, &data, key.clone())?;
                Ok((key, value))
            }
            None => Ok((mlua::Value::Nil, mlua::Value::Nil)),
        }
    })?;
    lua.set_app_data(Step(step.clone()));
    Ok(step)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What it read before: serde_json's tree, then a value of it.
    fn before(text: &str) -> Option<Value> {
        serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .map(crate::node::from_json)
    }

    #[test]
    fn json_reads_as_it_did_through_serde_jsons_tree() {
        let cases = [
            "null",
            "true",
            "false",
            "0",
            "-0",
            "1.5e300",
            "-12",
            "18446744073709551615",
            "9007199254740993",
            "1e400",
            "\"\"",
            "\"a\\u00e9\\n\\\"\\ud83d\\ude00\"",
            "[]",
            "{}",
            "[1,[2,[3,{}]],{\"a\":[]}]",
            "{\"b\":1,\"a\":2}",
            "{\"a\":1,\"a\":2,\"b\":3,\"a\":4}",
            "{\"\\u0061\":1,\"a\":2}",
            "  {\"k\" : [ true , null ] }  ",
            "{\"é\":\"日本\"}",
            "[1,]",
            "{\"a\":1,}",
            "[1] x",
            "",
            "{\"a\"}",
            "[",
            "01",
            "nan",
            "\"\\x\"",
            "{\"a\":{\"a\":{\"a\":{}}}}",
        ];
        for text in cases {
            assert_eq!(parse(text).ok(), before(text), "{text:?}");
        }
        // Bytes that aren't UTF-8 inside a string fail, as they did.
        let bad = b"[\"\xff\"]";
        assert!(parse_bytes(bad).is_err());
        assert!(serde_json::from_slice::<serde_json::Value>(bad).is_err());
        // Random documents, arrays and objects mixed and nested.
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        fn doc(next: &mut dyn FnMut(u64) -> u64, depth: u32) -> String {
            let keys = ["a", "b", "id", "text", "a", "é", "k\\\"q", ""];
            match if depth > 4 { next(5) } else { next(7) } {
                0 => "null".into(),
                1 => {
                    if next(2) == 0 {
                        "true".into()
                    } else {
                        "false".into()
                    }
                }
                2 => format!("{}", next(1 << 20) as f64 / 7.0 - 1000.0),
                3 => format!("{}", next(u64::MAX)),
                4 => format!("\"s{}\\n\"", next(100)),
                5 => {
                    let items: Vec<String> = (0..next(5)).map(|_| doc(next, depth + 1)).collect();
                    format!("[{}]", items.join(","))
                }
                _ => {
                    let items: Vec<String> = (0..next(6))
                        .map(|_| {
                            format!(
                                "\"{}\":{}",
                                keys[next(keys.len() as u64) as usize],
                                doc(next, depth + 1)
                            )
                        })
                        .collect();
                    format!("{{{}}}", items.join(","))
                }
            }
        }
        for _ in 0..3000 {
            let text = doc(&mut next, 0);
            assert_eq!(parse(&text).ok(), before(&text), "{text}");
        }
    }

    #[test]
    fn a_key_named_by_many_objects_is_one_string() {
        let value = parse(r#"[{"id":"a","x":1},{"id":"b"},{"id":"c","x":{"id":0}}]"#).unwrap();
        let ids: Vec<*const u8> = value
            .list()
            .iter()
            .map(|row| match row {
                Value::Map(fields) => fields[0].0.as_ptr(),
                _ => unreachable!(),
            })
            .collect();
        assert!(ids.windows(2).all(|w| w[0] == w[1]));
    }
}
