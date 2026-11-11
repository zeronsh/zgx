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

pub(crate) use Tok as TapeTok;
/// One value of a document read onto a tape (`decode_to_lua`), in order: a
/// list or a map its count first, then its items (a map's a key and a value
/// each). A string is its place in the tape's bytes.
#[derive(Clone, Copy)]
pub(crate) enum Tok {
    Nil,
    Bool(bool),
    Num(f64),
    Str(u32, u32),
    List(u32),
    Map(u32),
}
/// A document read flat: its values in order, its strings' bytes in one
/// buffer, and whether any map is empty (an object, marked so).
#[derive(Default)]
struct Tape {
    toks: Vec<Tok>,
    bytes: Vec<u8>,
    objects: bool,
}
impl Tape {
    fn string(&mut self, s: &str) -> Tok {
        let at = self.bytes.len() as u32;
        self.bytes.extend_from_slice(s.as_bytes());
        Tok::Str(at, s.len() as u32)
    }
}
struct OnTape<'a>(&'a mut Tape);
impl<'de> DeserializeSeed<'de> for OnTape<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, reader: D) -> Result<(), D::Error> {
        reader.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for OnTape<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("JSON")
    }
    fn visit_bool<E>(self, b: bool) -> Result<(), E> {
        self.0.toks.push(Tok::Bool(b));
        Ok(())
    }
    fn visit_i64<E>(self, n: i64) -> Result<(), E> {
        self.0.toks.push(Tok::Num(n as f64));
        Ok(())
    }
    fn visit_u64<E>(self, n: u64) -> Result<(), E> {
        self.0.toks.push(Tok::Num(n as f64));
        Ok(())
    }
    fn visit_f64<E>(self, n: f64) -> Result<(), E> {
        self.0.toks.push(Tok::Num(n));
        Ok(())
    }
    fn visit_str<E>(self, s: &str) -> Result<(), E> {
        let tok = self.0.string(s);
        self.0.toks.push(tok);
        Ok(())
    }
    fn visit_unit<E>(self) -> Result<(), E> {
        self.0.toks.push(Tok::Nil);
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut items: A) -> Result<(), A::Error> {
        let at = self.0.toks.len();
        self.0.toks.push(Tok::List(0));
        let mut n = 0_u32;
        while items.next_element_seed(OnTape(&mut *self.0))?.is_some() {
            n += 1;
        }
        self.0.toks[at] = Tok::List(n);
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<(), A::Error> {
        let at = self.0.toks.len();
        self.0.toks.push(Tok::Map(0));
        let mut n = 0_u32;
        while entries.next_key_seed(KeyOnTape(&mut *self.0))?.is_some() {
            entries.next_value_seed(OnTape(&mut *self.0))?;
            n += 1;
        }
        self.0.objects |= n == 0;
        self.0.toks[at] = Tok::Map(n);
        Ok(())
    }
}
struct KeyOnTape<'a>(&'a mut Tape);
impl<'de> DeserializeSeed<'de> for KeyOnTape<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, reader: D) -> Result<(), D::Error> {
        reader.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for KeyOnTape<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a key")
    }
    fn visit_str<E>(self, key: &str) -> Result<(), E> {
        let tok = self.0.string(key);
        self.0.toks.push(tok);
        Ok(())
    }
}

/// `text`, which must be JSON whole, as Luau tables, read onto a flat tape
/// and the tables made from it in one C call: no tree of values between
/// (`parse`, then `value::to_lua`, which built one, made the tables of it,
/// and dropped it). The same tables, counted the same, but that a key an
/// object gives twice counts twice (its last value kept, as ever).
pub(crate) fn decode_to_lua(lua: &mlua::Lua, text: &[u8]) -> Result<mlua::Value, DecodeError> {
    let mut tape = Tape::default();
    let mut reader = serde_json::Deserializer::from_slice(text);
    OnTape(&mut tape)
        .deserialize(&mut reader)
        .map_err(DecodeError::Json)?;
    reader.end().map_err(DecodeError::Json)?;
    crate::value::tape_to_lua(lua, &tape.toks, &tape.bytes, tape.objects).map_err(DecodeError::Lua)
}
/// Why `decode_to_lua` failed: the JSON, or making its tables.
pub(crate) enum DecodeError {
    Json(serde_json::Error),
    Lua(mlua::Error),
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

    /// A document's tables read back, and the tables and items counted.
    type Read = (Value, (u64, u64));

    /// `text` both ways: through a value (`parse`, then `to_lua`) and onto
    /// a tape (`decode_to_lua`): the tables read back as the same value, or
    /// both fail; and the tables and items each counted.
    fn both_ways(lua: &mlua::Lua, text: &str) -> Option<(Read, Read)> {
        use crate::value::{convert, tables_converted, to_lua, values_converted};
        let counted = |make: &dyn Fn() -> Option<mlua::Value>| {
            let (tables, items) = (tables_converted(), values_converted());
            let made = make()?;
            let counts = (tables_converted() - tables, values_converted() - items);
            Some((convert(made, None).unwrap(), counts))
        };
        let through_value = counted(&|| parse(text).ok().map(|v| to_lua(&v, lua, false).unwrap()));
        let on_tape = counted(&|| decode_to_lua(lua, text.as_bytes()).ok());
        assert_eq!(
            through_value.is_some(),
            on_tape.is_some(),
            "one failed: {text:?}"
        );
        through_value.zip(on_tape)
    }

    /// Read onto a tape, a document makes the tables it made through a
    /// value: every case above, random documents (objects naming a key
    /// twice among them, whose last value is kept), and documents shaped as
    /// a chat feed, whose tables and items are counted the same too.
    #[test]
    fn a_document_read_onto_a_tape_makes_the_tables_a_value_made() {
        let lua = mlua::Lua::new();
        for text in [
            "null",
            "true",
            "0",
            "-0",
            "1.5e300",
            "18446744073709551615",
            "\"\"",
            "\"a\\u00e9\\n\\\"\\ud83d\\ude00\"",
            "[]",
            "{}",
            "[1,[2,[3,{}]],{\"a\":[]}]",
            "{\"a\":1,\"a\":2,\"b\":3,\"a\":4}",
            "{\"é\":\"日本\"}",
            "[1,]",
            "{\"a\"}",
            "[",
            "01",
            "\"\\x\"",
        ] {
            if let Some(((a, _), (b, _))) = both_ways(&lua, text) {
                assert!(a == b, "{text:?}");
            }
        }
        // Bytes that aren't UTF-8 inside a string fail, as they did.
        assert!(decode_to_lua(&lua, b"[\"\xff\"]").is_err());
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
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
                1 => "true".into(),
                2 => format!("{}", next(1 << 20) as f64 / 7.0 - 1000.0),
                3 => format!("{}", next(u64::MAX)),
                4 => format!("\"s{}\\n\"", next(100)),
                5 => format!(
                    "[{}]",
                    (0..next(5))
                        .map(|_| doc(next, depth + 1))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                _ => format!(
                    "{{{}}}",
                    (0..next(6))
                        .map(|_| format!(
                            "\"{}\":{}",
                            keys[next(keys.len() as u64) as usize],
                            doc(next, depth + 1)
                        ))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
            }
        }
        for _ in 0..3000 {
            let text = doc(&mut next, 0);
            if let Some(((a, _), (b, _))) = both_ways(&lua, &text) {
                assert!(a == b, "{text}");
            }
        }
        // A feed of messages, as Expo Exact's heavy list's: each object's
        // keys its own, so its counts are the same too.
        let words = [
            "launch",
            "café",
            "日本語",
            "\\\"quoted\\\"",
            "line\\nbreak",
            "emoji 🎉",
            "",
        ];
        let feed: Vec<String> = (0..400)
            .map(|i| {
                let runs: Vec<String> = (0..1 + next(6))
                    .map(|_| {
                        let t = words[next(words.len() as u64) as usize];
                        match next(3) {
                            0 => format!("{{\"t\":\"{t} \"}}"),
                            _ => format!("{{\"s\":\"bold\",\"t\":\"{t}\"}}"),
                        }
                    })
                    .collect();
                let mut fields = vec![
                    format!("\"id\":\"m{i}\""),
                    format!(
                        "\"author\":\"{}\"",
                        words[next(words.len() as u64) as usize]
                    ),
                    format!("\"minutesAgo\":{}", next(30_000)),
                    format!("\"paragraphs\":[[{}]]", runs.join(",")),
                    "\"meta\":{}".into(),
                ];
                if next(3) == 0 {
                    fields.push(format!(
                        "\"photos\":[{{\"src\":\"p{}.jpg\",\"w\":1600,\"h\":1200}}]",
                        next(60)
                    ));
                }
                if next(4) == 0 {
                    fields.push(format!(
                        "\"reactions\":[{{\"emoji\":\"👍\",\"count\":{}}}]",
                        next(40)
                    ));
                }
                format!("{{{}}}", fields.join(","))
            })
            .collect();
        let text = format!("{{\"version\":1,\"messages\":[{}]}}", feed.join(","));
        let ((a, counted), (b, taped)) = both_ways(&lua, &text).expect("the feed reads");
        assert!(a == b, "the feed's tables");
        assert_eq!(counted, taped, "the feed's tables and items, counted");
        // Its empty objects stay objects.
        let made = decode_to_lua(&lua, text.as_bytes()).ok().unwrap();
        let mlua::Value::Table(root) = made else {
            panic!("a table")
        };
        let messages: mlua::Table = root.raw_get("messages").unwrap();
        let first: mlua::Table = messages.raw_get(1).unwrap();
        let meta: mlua::Table = first.raw_get("meta").unwrap();
        assert!(meta.metatable().is_some(), "an empty object, marked one");
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
