# A value's strings, lists and maps

`zgx_script::Value` holds its strings, lists and maps in types of zgx's own, laid out as zgx states and not as the compiler chooses: a `Str` (a counted, UTF-8 slice), a `List` (`Slice<Value>`) and a `Map` (`Slice<Field>`, a field a `Str` key and a `Value`, sorted by key). A `Slice<T>` is a counted slice as `Rc<[T]>` is, with its counts and items at stated places, so code built apart from zgx (native modules, a change to come) can read a value where it is. Reading one costs what reading the `Rc` did, and every Luau scene of the perf gate runs at or under what it cost before.

## Moving Rust code to them

`Value`'s variants hold other types than they did, so Rust that matches on them or builds them by hand changes. `Value`'s functions are as they were: `Value::record`, `Value::from`, `field`, `list`, `items`, `at`, `set_path`, `to_json`, `from_json`.

| | Was | Is |
| --- | --- | --- |
| `Value::Str` | `Rc<str>` | `Str` |
| `Value::List` | `Rc<[Value]>` | `List` (`Slice<Value>`) |
| `Value::Map` | `Rc<[(Rc<str>, Value)]>` | `Map` (`Slice<Field>`) |
| a map's field | `(Rc<str>, Value)` | `Field(Str, Value)` |

What keeps compiling: reading a string as `&str` (`&**s`, `s.len()`, comparing with a `&str`), a list as a slice (`items.iter()`, `items[i]`, `items.len()`), `field.0` and `field.1`, `Value::List(vec.into())`, `Value::List(iter.collect())`, `Value::Str(text.into())` from a `&str` or a `String`, and `Value::Map(vec![("key".into(), value)].into())`.

What changes, each a compile error with the fix beside it:

| Pattern | Before | After |
| --- | --- | --- |
| a value's string kept | `Value::Str(s) => s.clone()` into an `Rc<str>` | keep a `Str`: the holder's type changes, `s.clone()` stays |
| a set or a map keyed by a value's string | `HashSet<Rc<str>>`, `HashMap<Rc<str>, T>` | `HashSet<Str>`, `HashMap<Str, T>` (looked up by `&str`, as before) |
| a value from an `Rc<str>` by the variant as a function | `label.map_or(Value::Nil, Value::Str)` | `label.map_or(Value::Nil, Value::from)` (a copy of the text) |
| a pair pattern over a map's fields | `for (key, value) in fields.iter()` | `for Field(key, value) in fields.iter()` |
| a field built as a pair where a `Field` is wanted | `fields.push((key, value))` | `fields.push(Field(key, value))` |
| `Rc`'s functions on a string, a list or a map | `Rc::ptr_eq(a, b)`, `Rc::get_mut(items)`, `Rc::strong_count`, `Rc::downgrade` | `Slice::ptr_eq(a, b)`, `Slice::get_mut(items)`, … (`Str::ptr_eq` for strings) |
| a list or a map made through `Rc` | `Value::List(Rc::from(rows))` | `Value::List(rows.into())` |
| a type naming the old payload | `Rc<[Value]>`, `Vec<(Rc<str>, Value)>` | `List`, `Vec<Field>` (or `Vec<(Str, Value)>`) |
| `Value::folded`'s id | an `Rc<str>` | anything a `Str` is made from (`&str`, `String`, `Str`) |

**Two conversions copy**, and exist so that code which needs the other type can have it: a `Str` to or from an `Rc<str>` (`Str::rc`, `Str::from(rc)`), and `Slice::from(Rc<[T]>)`. Neither is wrong to use; on a hot path, hold a `Str` or a `List` from the start. `Str::rc` is the one to search for after a port: each is a copy that was a count.

How many sites, counted from the changes themselves:

| | Lines changed | Of |
| --- | --- | --- |
| zgx's own crates, outside the new types (zgx-script, its tests and examples, zgx-desktop's examples) | 460, in 36 files | about 634,000 lines of Rust |
