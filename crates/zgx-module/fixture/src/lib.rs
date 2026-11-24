//! A native module for zgx-script's tests (`tests/native_modules.rs`).

use zgx_module::{Host, Value};

zgx_module::register!("fixture", |host: &Host| {
    // What it was given, back: every kind of value there and back.
    host.function("fixture.echo", |arg| arg.to_owned());
    // The same, handed its argument: nothing counted.
    host.function_taking("fixture.echo_taking", |arg| arg);
    // `{ words, longest, kinds }` of a list of strings.
    host.function("fixture.words", |texts| {
        let words: usize = texts
            .items()
            .map(|t| t.str().map_or(0, |s| s.split_whitespace().count()))
            .sum();
        let longest = texts
            .items()
            .filter_map(|t| t.str())
            .max_by_key(|s| s.len())
            .unwrap_or("");
        Value::record([
            ("words", Value::from(words as f64)),
            ("longest", Value::from(longest)),
            ("first", texts.item(0).map_or(Value::NIL, Value::from)),
        ])
    });
    // A map's fields as `{ key, value }` pairs, in its order, less `drop`.
    host.function("fixture.fields", |arg| {
        let drop = arg.field("drop").and_then(|d| d.str()).unwrap_or("");
        Value::list(
            arg.fields()
                .filter(|(k, _)| *k != drop)
                .map(|(k, v)| Value::record([("key", Value::from(k)), ("value", v.to_owned())])),
        )
    });
    // A list's numbers summed (a pass over more items than one read holds),
    // and its items `from` to `to` as a list.
    host.function("fixture.sum", |list| {
        Value::from(list.items().filter_map(|n| n.num()).sum::<f64>())
    });
    host.function("fixture.slice", |arg| {
        let [list, from, to, none] = arg.get(["list", "from", "to", "none"]);
        assert!(none.is_none());
        let at = |n: Option<zgx_module::Ref>| n.and_then(|n| n.num()).unwrap_or(0.) as usize;
        list.map_or(Value::NIL, |list| list.slice(at(from), at(to)))
    });
    // How many fields a map has, its last key, and fields found and not
    // found in it: a map wide enough to be searched by halves.
    host.function("fixture.wide", |map| {
        let last = map.fields().last().map_or("", |(k, _)| k);
        let found = map.field("k57").and_then(|n| n.num()).unwrap_or(0.);
        let none =
            map.field("k570").is_none() && map.field("a").is_none() && map.field("z").is_none();
        Value::record([
            ("n", Value::from(map.len() as f64)),
            ("last", Value::from(last)),
            ("k57", Value::from(found)),
            ("none", Value::from(none)),
        ])
    });
    // A string's length: an argument read and a number made, so what
    // entering a module costs and little else.
    host.function("fixture.length", |text| {
        Value::from(text.str().map_or(0, str::len) as f64)
    });
    // The same of a string of any length, one the runtime holds in pieces
    // too; and whether `str` alone reads it.
    host.function("fixture.long", |text| {
        Value::record([
            (
                "len",
                Value::from(text.text().map_or(0, |t| t.len()) as f64),
            ),
            ("str", Value::from(text.str().is_some())),
        ])
    });
    // Rows read a field at a time, as an app's model code reads them: what
    // `module_call_bench` compares with the same pass linked in.
    host.function("fixture.rows", |rows| {
        let mut sum = 0.;
        for row in rows.items() {
            let [id, text, n, done] = row.get(["id", "text", "n", "done"]);
            sum += n.and_then(|n| n.num()).unwrap_or(0.);
            sum += (id.and_then(|s| s.str()).map_or(0, str::len)
                + text.and_then(|s| s.str()).map_or(0, str::len)) as f64;
            sum += f64::from(u8::from(done.and_then(|d| d.bool()) == Some(true)));
        }
        Value::from(sum)
    });
    // A value kept by an action, read by a function: a module's own state,
    // one for each thread.
    let kept = host.state::<Value>();
    host.action("fixture.keep", move |arg| {
        // (What it held is freed outside `with`.)
        let was = kept.with(|kept| std::mem::replace(kept, arg.to_owned()));
        drop(was);
    });
    host.function("fixture.kept", move |_| kept.with(|kept| kept.clone()));
});
