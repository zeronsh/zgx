//! What a full collection leaves, and who holds it: a heap dump's objects
//! named by what they are, the ones that came after a mark, and for the
//! objects of a kind the chain of holders from a root (`zgx inspect gc`).
use serde_json::Value as Json;
use std::collections::{HashMap, HashSet, VecDeque};

/// What `zgx inspect gc` asks of the heap beyond its totals.
#[derive(Debug, Clone, Default)]
pub struct GcQuery {
    /// Rows to show in each list.
    pub top: usize,
    /// Objects of this kind (`State`, a userdata type; `function`, `table`;
    /// or part of a function's name or chunk) to find the holders of.
    pub retained: Option<String>,
    /// How many holder paths to show.
    pub sample: usize,
    /// Remember the objects there are now.
    pub mark: bool,
    /// Only the objects that weren't there at the mark.
    pub since_mark: bool,
}

/// A group of retained objects sharing one holder path.
#[derive(Debug, Clone, Default)]
pub struct GcHolder {
    /// How many objects have this path.
    pub count: usize,
    /// From a root to the object: a hop a line.
    pub path: Vec<String>,
}

/// What a query found in one VM.
#[derive(Debug, Clone, Default)]
pub struct GcFound {
    /// Objects the mark remembered (when asked to mark).
    pub marked: Option<usize>,
    /// `since_mark` was asked and no mark had been made.
    pub no_mark: bool,
    /// The objects added since the mark, by kind: name, objects, bytes.
    pub since: Vec<(String, usize, u64)>,
    /// How many objects matched `retained`, and the paths holding them.
    pub matched: usize,
    /// The commonest holder paths, most first.
    pub holders: Vec<GcHolder>,
}

type Objects = serde_json::Map<String, Json>;

fn field<'a>(objects: &'a Objects, addr: &str, key: &str) -> Option<&'a str> {
    let pairs = objects.get(addr)?["pairs"].as_array()?;
    pairs.chunks(2).find_map(|kv| {
        let k = objects.get(kv.first()?.as_str()?)?;
        (k["type"] == "string" && k["data"] == key)
            .then(|| objects.get(kv.get(1)?.as_str()?)?["data"].as_str())
            .flatten()
    })
}

fn proto_of<'a>(objects: &'a Objects, function: &'a Json) -> Option<&'a Json> {
    objects.get(function["proto"].as_str()?)
}

/// An object as a person names it: `userdata State`, `function name @chunk:12`.
fn describe(objects: &Objects, addr: &str) -> String {
    let Some(object) = objects.get(addr) else {
        return "?".into();
    };
    match object["type"].as_str().unwrap_or("?") {
        "userdata" => {
            let kind = object["metatable"]
                .as_str()
                .and_then(|meta| field(objects, meta, "__type"))
                .unwrap_or("unknown");
            format!("userdata {kind}")
        }
        "function" => match proto_of(objects, object) {
            Some(proto) => format!(
                "function {} @{}:{}",
                object["name"].as_str().unwrap_or("?"),
                proto["source"]
                    .as_str()
                    .map_or("?", |s| s.trim_start_matches('@')),
                proto["line"].as_i64().unwrap_or(0)
            ),
            None => format!("function {} [C]", object["name"].as_str().unwrap_or("?")),
        },
        "string" => format!("string {:?}", object["data"].as_str().unwrap_or("")),
        other => other.to_string(),
    }
}

/// The objects `addr` holds, each with how it is held.
fn edges<'a>(objects: &'a Objects, addr: &str) -> Vec<(String, &'a str)> {
    let Some(object) = objects.get(addr) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match object["type"].as_str().unwrap_or("") {
        "table" => {
            if let Some(pairs) = object["pairs"].as_array() {
                for kv in pairs.chunks(2) {
                    let key = kv.first().and_then(Json::as_str);
                    let value = kv.get(1).and_then(Json::as_str);
                    let name = match key.and_then(|k| objects.get(k)) {
                        Some(k) if k["type"] == "string" => {
                            format!(".{}", k["data"].as_str().unwrap_or("?"))
                        }
                        Some(k) => format!("[{}]", k["type"].as_str().unwrap_or("?")),
                        None => "[?]".to_string(),
                    };
                    if let Some(value) = value {
                        out.push((name.clone(), value));
                    }
                    if let Some(key) = key {
                        out.push((format!("key {name}"), key));
                    }
                }
            }
            for item in object["array"].as_array().into_iter().flatten() {
                if let Some(item) = item.as_str() {
                    out.push(("[n]".to_string(), item));
                }
            }
            if let Some(meta) = object["metatable"].as_str() {
                out.push(("metatable".to_string(), meta));
            }
        }
        "function" => {
            for (i, up) in object["upvalues"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                if let Some(up) = up.as_str() {
                    out.push((format!("upvalue#{i}"), up));
                }
            }
        }
        "upvalue" => {
            if let Some(value) = object["object"].as_str() {
                out.push(("value".to_string(), value));
            }
        }
        "thread" => {
            for item in object["stack"].as_array().into_iter().flatten() {
                if let Some(item) = item.as_str() {
                    out.push(("stack".to_string(), item));
                }
            }
        }
        _ => {}
    }
    out
}

fn hop(objects: &Objects, label: &str, addr: &str) -> Option<String> {
    let type_ = objects.get(addr)?["type"].as_str()?;
    (type_ != "upvalue").then(|| format!("{label} → {}", describe(objects, addr)))
}

/// Answer `query` for the dump `json` of a VM whose earlier mark is `mark`
/// (replaced by this heap's objects when asked to mark).
pub fn find(json: &str, query: &GcQuery, mark: &mut Option<HashSet<String>>) -> GcFound {
    let mut found = GcFound::default();
    let Ok(Json::Object(mut root)) = serde_json::from_str::<Json>(json) else {
        return found;
    };
    let Some(Json::Object(objects)) = root.remove("objects") else {
        return found;
    };
    let roots: Vec<String> = root["roots"]
        .as_object()
        .into_iter()
        .flat_map(|r| r.values().filter_map(|v| v.as_str().map(String::from)))
        .collect();
    let fresh = |addr: &str| match (&query.since_mark, mark.as_ref()) {
        (true, Some(old)) => !old.contains(addr),
        _ => true,
    };
    if query.since_mark {
        if mark.is_none() {
            found.no_mark = true;
        } else {
            let mut kinds: HashMap<String, (usize, u64)> = HashMap::new();
            for (addr, object) in &objects {
                if fresh(addr) && object["type"] != "proto" && object["type"] != "string" {
                    let entry = kinds.entry(describe(&objects, addr)).or_default();
                    entry.0 += 1;
                    entry.1 += object["size"].as_u64().unwrap_or(0);
                }
            }
            let mut kinds: Vec<_> = kinds.into_iter().map(|(n, (c, b))| (n, c, b)).collect();
            kinds.sort_by_key(|(n, c, _)| (std::cmp::Reverse(*c), n.clone()));
            found.since = kinds;
        }
    }
    if let Some(wanted) = query.retained.as_deref() {
        let matches = |addr: &str| {
            let kind = objects[addr]["type"].as_str().unwrap_or("");
            kind != "proto" && (kind == wanted || describe(&objects, addr).contains(wanted))
        };
        let mut parent: HashMap<&str, (&str, String)> = HashMap::new();
        let mut seen: HashSet<&str> = roots.iter().map(String::as_str).collect();
        let mut queue: VecDeque<&str> = roots.iter().map(String::as_str).collect();
        let mut targets = Vec::new();
        while let Some(at) = queue.pop_front() {
            if at != "0" && fresh(at) && objects.contains_key(at) && matches(at) {
                targets.push(at);
            }
            for (label, to) in edges(&objects, at) {
                if seen.insert(to) {
                    parent.insert(to, (at, label));
                    queue.push_back(to);
                }
            }
        }
        found.matched = targets.len();
        let mut groups: HashMap<Vec<String>, usize> = HashMap::new();
        for target in targets {
            let mut path = Vec::new();
            let mut at = target;
            while let Some((from, label)) = parent.get(at) {
                if let Some(line) = hop(&objects, label, at) {
                    path.push(line);
                }
                at = from;
            }
            let root = roots
                .iter()
                .position(|r| r == at)
                .and_then(|_| root["roots"].as_object())
                .and_then(|r| r.iter().find(|(_, v)| v.as_str() == Some(at)))
                .map_or("root", |(name, _)| name.as_str());
            path.push(root.to_string());
            path.reverse();
            *groups.entry(path).or_default() += 1;
        }
        let mut groups: Vec<_> = groups
            .into_iter()
            .map(|(path, count)| GcHolder { count, path })
            .collect();
        groups.sort_by_key(|g| (std::cmp::Reverse(g.count), g.path.clone()));
        groups.truncate(query.sample.max(1));
        found.holders = groups;
    }
    if query.mark {
        let all: HashSet<String> = objects.keys().cloned().collect();
        found.marked = Some(all.len());
        *mark = Some(all);
    }
    found
}
