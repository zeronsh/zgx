//! `ui.json.data` and `ui.json.load`: read-only lists and objects a script
//! reads as it reads `ui.json.decode`'s tables (indexing, `#`, `for … in`),
//! their parts made as they are read, and state again without a walk.
mod common;
use common::Harness;
use zgx_script::{Bundle, BundleFile, ScriptHost, build};

/// Random JSON documents, large enough that their tables are filled a level
/// at a time: arrays and objects nested, and nulls among items and fields
/// if `nulls`.
fn documents(n: usize, nulls: bool) -> Vec<String> {
    let mut seed = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    fn doc(next: &mut dyn FnMut(u64) -> u64, nulls: bool, depth: u32) -> String {
        let keys = ["a", "b", "id", "text", "é", "k q", "1", "n"];
        match if depth > 3 { next(5) } else { next(8) } {
            0 => if nulls { "null" } else { "0" }.into(),
            1 => ["true", "false"][next(2) as usize].into(),
            2 => format!("{}", next(1 << 20) as f64 / 8.0 - 1000.0),
            3 => format!("{}", next(1000)),
            4 => format!("\"s{}\\n\\u00e9\"", next(100)),
            5 | 6 => {
                let items: Vec<String> =
                    (0..next(24)).map(|_| doc(next, nulls, depth + 1)).collect();
                format!("[{}]", items.join(","))
            }
            _ => {
                let items: Vec<String> = (0..next(8))
                    .map(|_| {
                        format!(
                            "\"{}\":{}",
                            keys[next(keys.len() as u64) as usize],
                            doc(next, nulls, depth + 1)
                        )
                    })
                    .collect();
                format!("{{{}}}", items.join(","))
            }
        }
    }
    (0..n)
        .map(|i| match i % 3 {
            // An array or an object at the top, as data mostly is.
            0 => format!(
                "[{}]",
                (0..20)
                    .map(|_| doc(&mut next, nulls, 1))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            _ => doc(&mut next, nulls, 0),
        })
        .collect()
}

/// A walk of a list or object that says everything a script can read of
/// it: `for k, v in t`, each value by its key, and `#` (with `exact`: a null
/// among a list's items is a hole in `decode`'s tables, where `#` may stop),
/// first reading it as `first` does.
const WALK: &str = r#"
local deep = 0
local function walk(v, first, exact, depth)
    depth = depth or 0
    if type(v) ~= "table" and type(v) ~= "userdata" then
        return tostring(v)
    end
    if type(v) == "userdata" and depth > 0 then
        deep += 1
    end
    if first == "len" then
        local _ = #v
    elseif first == "index" then
        local _ = v[1], v.a
    end
    local keys, same = {}, {}
    for k, item in v do
        table.insert(keys, k)
        -- What the loop gave is what indexing gives: the same part.
        same[k] = item == v[k]
    end
    table.sort(keys, function(a, b)
        return `{typeof(a)}{a}` < `{typeof(b)}{b}`
    end)
    local out = { if exact then `#{#v}` else "" }
    for _, k in keys do
        table.insert(out, `{typeof(k)}{k}={same[k]}/{walk(v[k], first, exact, depth + 1)}`)
    end
    return "{" .. table.concat(out, ",") .. "}"
end
"#;

#[test]
fn json_data_reads_as_decode_does() {
    // Sized to run in well under a third of a debug build's load budget
    // on a loaded runner.
    let mut docs = documents(160, true);
    docs.extend(documents(80, false));
    // Large, so their parts are lists and objects of their own (`JsonData`)
    // a few levels down, not only tables.
    for nulls in [true, false] {
        let parts = documents(160, nulls);
        for chunk in parts.chunks(40) {
            let (a, b) = chunk.split_at(20);
            docs.push(format!(
                r#"{{"a":[{}],"b":{{"c":[{}],"n":1}}}}"#,
                a.join(","),
                b.join(",")
            ));
        }
    }
    let list: Vec<String> = docs.iter().map(|d| format!("[==[{d}]==]")).collect();
    let code = format!(
        r#"{WALK}
local docs = {{ {} }}
local firsts = {{ "iter", "len", "index" }}
local differ = {{}}
for i, text in docs do
    local first = firsts[i % #firsts + 1]
    local exact = not string.find(text, "null", 1, true)
    local data, decoded = ui.json.data(text), ui.json.decode(text)
    local a, b = walk(data, first, exact), walk(decoded, first, exact)
    if a ~= b then
        table.insert(differ, `{{i}} ({{first}}): {{text}}\n data   {{a}}\n decode {{b}}`)
    end
    -- And as state, the same value (where decode's tables hold it all).
    if exact and ui.json.encode(data) ~= ui.json.encode(decoded) then
        table.insert(differ, `{{i}}: encodes otherwise: {{text}}`)
    end
end
return ui.column {{
    ui.text {{ id = "differ", if #differ == 0 then "none" else table.concat(differ, "\n") }},
    ui.text {{ id = "deep", tostring(deep) }},
}}"#,
        list.join(",\n")
    );
    let mut h = Harness::inline(&code);
    assert_eq!(h.error(), None);
    assert_eq!(h.text("differ"), "none");
    // Lists and objects below the top were read as `JsonData` too.
    let deep: usize = h.text("deep").parse().unwrap();
    assert!(deep > 100, "{deep} parts below the top were JsonData");
}

#[test]
fn json_data_is_state_without_being_made_into_tables() {
    let rows: Vec<String> = (0..2000)
        .map(|i| format!(r#"{{"id":"m{i}","text":"message {i}","likes":{i},"tags":["a","b"]}}"#))
        .collect();
    let code = format!(
        r#"local data = ui.json.data([==[{{"messages":[{}],"title":"feed"}}]==])
local feed = ui.state("feed", data.messages)
return ui.column {{
    ui.text {{ id = "title", data.title }},
    ui.text {{ id = "same", tostring(data.messages == data.messages and data == data) }},
    -- Nulls among items stay (decode's tables have holes there).
    ui.text {{ id = "nulls", ui.json.encode(ui.json.data(
        "[1,null,3,null,5,6,7,8,9,10,11,12,13,14,15,16,17]"
    )) }},
    ui.list {{ data = feed, key = "id", height = 20, row = function(row)
        return ui.text {{ id = `row-{{row.id}}`, row.text }}
    end }},
}}"#,
        rows.join(",")
    );
    let tables = zgx_script::tables_converted();
    let mut h = Harness::inline(&code);
    h.settle();
    assert_eq!(h.error(), None);
    assert_eq!(h.text("title"), "feed");
    assert_eq!(h.text("same"), "true", "a part read twice is the same");
    assert_eq!(
        h.text("nulls"),
        "[1,null,3,null,5,6,7,8,9,10,11,12,13,14,15,16,17]"
    );
    assert_eq!(h.text("row-m0"), "message 0", "the list shows its rows");
    let made = zgx_script::tables_converted() - tables;
    // The rows shown, not 2,000 (nor the 4,000 tables of their tags).
    assert!(made < 200, "{made} tables made");
}

#[test]
fn writing_json_data_names_decode_and_tables_only_functions_say_what_it_is() {
    // Lists in a list: `JsonData` (a list of plain values is a table).
    let big: Vec<String> = (0..600).map(|i| format!("[{i}]")).collect();
    let code = format!(
        r#"local data = ui.json.data([==[{{"rows":[1,2,3],"n":1,"big":[{}]}}]==])
local function says(f) return tostring(select(2, pcall(f))) end
return ui.column {{
    ui.text {{ id = "ipairs", says(function() for _ in ipairs(data.big :: any) do end end) }},
    ui.text {{ id = "table", says(function() return table.concat(data.big :: any) end) }},
    ui.text {{ id = "big", `{{#data.big}} {{data.big[600][1]}} {{typeof(data.big)}} {{typeof(data.big[1])}}` }},"#,
        big.join(",")
    ) + r#"
    ui.text { id = "new_key", says(function() (data :: any).other = 1 end) },
    ui.text { id = "field", says(function() (data :: any).n = 2 end) },
    ui.text { id = "item", says(function() (data.rows :: any)[1] = 5 end) },
    ui.text { id = "n", tostring(data.n) },
    ui.text { id = "type", typeof(data) },
    ui.text { id = "pairs", says(function() for _ in pairs(data :: any) do end end) },
    ui.text { id = "plain", (function()
        local n = 0
        for _, v in pairs({ a = 1, b = 2 }) do n += v end
        for _, v in ipairs({ 4, 5 }) do n += v end
        return tostring(n)
    end)() },
}"#;
    let mut h = Harness::inline(&code);
    assert_eq!(h.error(), None);
    for id in ["new_key", "field"] {
        assert!(
            h.text(id).contains("ui.json.decode"),
            "{id}: {}",
            h.text(id)
        );
    }
    // A small part is a read-only table (uncaught, the hint names decode).
    assert!(h.text("item").contains("readonly"), "{}", h.text("item"));
    assert_eq!(h.text("n"), "1");
    assert_eq!(h.text("type"), "JsonData");
    // Loud, never an empty loop.
    for id in ["pairs", "ipairs", "table"] {
        assert!(
            h.text(id).contains("table expected, got JsonData"),
            "{id}: {}",
            h.text(id)
        );
    }
    assert_eq!(h.text("big"), "600 599 JsonData table");
    assert_eq!(
        h.text("plain"),
        "12",
        "Luau's own pairs and ipairs, unchanged"
    );
    // Uncaught, a write to a part names decode too.
    let runtime = zgx::reactive::Runtime::new();
    let error = ScriptHost::inline(
        &runtime,
        "app",
        r#"local data = ui.json.data([==[{"rows":[1,2,3]}]==])
data.rows[1] = 5
return ui.text { "" }"#,
    )
    .reload()
    .unwrap_err();
    assert!(error.contains("ui.json.decode"), "{error}");
}

/// A bundle of `(path, text)` files, scripts compiled as a build would.
fn bundle(name: &'static str, files: &[(&'static str, &str)]) -> &'static Bundle {
    let mut files: Vec<BundleFile> = files
        .iter()
        .map(|(path, text)| BundleFile {
            path,
            bytes: Box::leak(
                if path.ends_with(".luau") {
                    build::compile(text.as_bytes()).unwrap()
                } else {
                    text.as_bytes().to_vec()
                }
                .into_boxed_slice(),
            ),
        })
        .collect();
    files.sort_by_key(|file| file.path);
    Box::leak(Box::new(Bundle {
        name,
        root: Box::leak(format!("/zgx-bundle/{name}").into_boxed_str()),
        files: Box::leak(files.into_boxed_slice()),
    }))
}

#[test]
fn ui_json_load_finds_its_file_as_require_finds_a_module() {
    let app = bundle(
        "json-load",
        &[
            (
                "app.luau",
                r#"local feed = require("./lib/feed")
local again = ui.json.load("data/feed.json")
local outside = select(2, pcall(ui.json.load, "../elsewhere.json"))
local broken = select(2, pcall(ui.json.load, "./data/broken"))
local folder = select(2, pcall(ui.json.load, "./data"))
return ui.column {
    ui.text { id = "first", feed.items[1].name },
    ui.text { id = "count", tostring(#feed.items) },
    ui.text { id = "same", tostring(feed == again) },
    ui.text { id = "outside", tostring(outside) },
    ui.text { id = "broken", tostring(broken) },
    ui.text { id = "folder", tostring(folder) },
}"#,
            ),
            // Relative to the module loading it, as `require` is.
            ("lib/feed.luau", r#"return ui.json.load("../data/feed")"#),
            (
                "data/feed.json",
                r#"{"items":[{"name":"one"},{"name":"two"},{"name":"three"}]}"#,
            ),
            ("data/broken.json", r#"{"items":[1,2"#),
        ],
    );
    let mut h = Harness::new(|runtime| ScriptHost::bundled(runtime, app, "app.luau"));
    assert_eq!(h.host.error().get(), None);
    assert_eq!(h.text("first"), "one");
    assert_eq!(h.text("count"), "3");
    assert_eq!(h.text("same"), "true", "read once per load");
    for (id, says) in [
        ("outside", "the script's directory"),
        ("broken", "broken.json"),
        ("folder", "a folder"),
    ] {
        let text = h.text(id);
        assert!(
            text.contains("ui.json.load(") && text.contains(says),
            "{id}: {text}"
        );
    }
}
