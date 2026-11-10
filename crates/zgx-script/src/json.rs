//! JSON read straight into script values: no tree of serde_json's in
//! between, and each object key one string however many objects name it
//! (10,000 messages' `id` share one).

use crate::value::Value;
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
