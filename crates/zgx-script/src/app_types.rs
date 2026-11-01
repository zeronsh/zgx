//! `zgx types <dir>`: zgx's definitions with an app's own theme tokens in
//! them, for editors to complete and check them.
//!
//! It writes `<dir>/.zgx/types.d.luau`: `zgx.d.luau` with `Tokens` (what
//! `ui.tokens` holds) grown by the names the app declares with literal
//! `ui.theme.tokens { … }` tables, and those names in `ui.theme.get`'s.
//! It is all of zgx's definitions, not a second file beside them:
//! luau-lsp reads a second file's types too late or too early for
//! `zgx.d.luau` to name them. An editor lists it in place of
//! `zgx.d.luau`; `zgx check` writes it and checks with it.
//!
//! `ui.tokens.notes.wash` is a field path, so a typo is a type error and
//! the editor completes the next name; a value is a plain `string`, so
//! props stay `string` (a union of 100 token names in `Color` made the
//! solver report false errors and take twice as long). `"$notes.wash"`
//! strings stay valid, and the `unknown-token` lint checks them.
use crate::check::{CheckError, DEFINITIONS, scripts};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `MessageKey` in `zgx.d.luau`, which an app's keys replace.
const MESSAGE_KEY_PLACEHOLDER: &str = "export type MessageKey = string";
/// The most keys `MessageKey` names: a longer union is past what luau-lsp
/// takes ("Exceeded allowed type length" by 2,000); a larger catalog keeps
/// `string`, and the `i18n-key` lint still names a key it lacks.
const MESSAGE_KEYS: usize = 1000;

/// Where the types go in an app's directory.
pub fn types_file(dir: &Path) -> PathBuf {
    dir.join(".zgx").join("types.d.luau")
}

/// A group of `ui.tokens`, as its type is written.
#[derive(Default)]
struct Group {
    /// Its tokens and groups, by the part of the name.
    members: BTreeMap<String, Member>,
    /// Holds names the lint can't read: any field is allowed.
    open: bool,
}
enum Member {
    Token,
    Group(Group),
}
impl Group {
    fn insert(&mut self, name: &str) {
        let (part, rest) = match name.split_once('.') {
            Some((part, rest)) => (part, Some(rest)),
            None => (name, None),
        };
        match rest {
            None => {
                match self.members.get_mut(part) {
                    // A group that is a token too: its own as `value`.
                    Some(Member::Group(group)) => group.insert("value"),
                    _ => {
                        self.members.insert(part.to_owned(), Member::Token);
                    }
                }
            }
            Some(rest) => {
                let member = self
                    .members
                    .entry(part.to_owned())
                    .or_insert_with(|| Member::Group(Group::default()));
                if matches!(member, Member::Token) {
                    let mut group = Group::default();
                    group.insert("value");
                    *member = Member::Group(group);
                }
                if let Member::Group(group) = member {
                    group.insert(rest);
                }
            }
        }
    }
    fn open(&mut self, name: &str) {
        if name.is_empty() {
            self.open = true;
            return;
        }
        let (part, rest) = name.split_once('.').unwrap_or((name, ""));
        let member = self
            .members
            .entry(part.to_owned())
            .or_insert_with(|| Member::Group(Group::default()));
        if matches!(member, Member::Token) {
            let mut group = Group::default();
            group.insert("value");
            *member = Member::Group(group);
        }
        if let Member::Group(group) = member {
            group.open(rest);
        }
    }
    /// Its type: fields by name; numbers as an array's (`space[3]`).
    fn write(&self, depth: usize, out: &mut String) {
        let numbered = self.members.keys().filter(|k| is_index(k)).count();
        if numbered > 0 && numbered == self.members.len() && !self.open {
            out.push_str("{ string }");
            return;
        }
        let indent = "\t".repeat(depth + 1);
        out.push_str("{\n");
        for (key, member) in self.members.iter().filter(|(k, _)| !is_index(k)) {
            out.push_str(&indent);
            // A keyword (`end`) is a field only quoted.
            match KEYWORDS.contains(&key.as_str()) {
                true => out.push_str(&format!("[\"{key}\"]")),
                false => out.push_str(key),
            }
            out.push_str(": ");
            match member {
                Member::Token => out.push_str("string"),
                Member::Group(group) => group.write(depth + 1, out),
            }
            out.push_str(",\n");
        }
        if self.open {
            out.push_str(&format!("{indent}[string]: any,\n"));
        } else if numbered > 0 {
            out.push_str(&format!("{indent}[number]: string,\n"));
        }
        out.push_str(&"\t".repeat(depth));
        out.push('}');
    }
}
/// Luau's reserved words: no field is named one bare.
const KEYWORDS: [&str; 21] = [
    "and", "break", "continue", "do", "else", "elseif", "end", "false", "for", "function", "if",
    "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until",
];
fn is_index(part: &str) -> bool {
    !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())
}
/// Whether a name can be a Luau field (`notes.wash`); others (`"my-card"`)
/// stay reachable as `"$…"` strings only.
fn is_field_path(name: &str) -> bool {
    name.split('.').all(|part| {
        is_index(part)
            || (part.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
    })
}

/// `Tokens`: the type of `ui.tokens` for zgx's standard tokens and
/// `extra` names, `open` groups taking any field.
fn tokens_type(extra: &[String], open: &[String], all_open: bool) -> String {
    let mut root = Group::default();
    for (_, name, _) in zgx::theme::standard() {
        root.insert(name);
    }
    for name in extra.iter().filter(|name| is_field_path(name)) {
        root.insert(name);
    }
    for group in open {
        root.open(group);
    }
    root.open |= all_open;
    let mut out = String::from("export type Tokens = ");
    root.write(0, &mut out);
    out.push('\n');
    out
}
/// `Tokens` for zgx's standard tokens alone, as `zgx.d.luau` has it.
pub fn standard_tokens_type() -> String {
    tokens_type(&[], &[], false)
}

/// Past this many targets the union is left a `string`: luau-lsp gives up
/// on unions much larger (sol's i18n keys, measured), and the
/// `unknown-target` lint checks them instead.
const TARGETS_MAX: usize = 1000;

/// `HookName` and `HookTarget` for the hooks and ids `targets` holds, in
/// place of zgx's `string`s, and `Selector` made a type of its own, so a
/// target misspelled, or a selector not written `ui.selector(…)`, is an
/// error; unchanged (all `string`s) for none, or past `TARGETS_MAX`.
fn hook_types(text: String, targets: &crate::hook_targets::Targets) -> String {
    const TARGET: &str = "export type HookTarget = string\n";
    const SELECTOR: &str = "export type Selector = string\n";
    assert!(
        text.contains(TARGET) && text.contains(SELECTOR),
        "zgx.d.luau declares HookTarget and Selector as strings"
    );
    if targets.len() == 0 || targets.len() > TARGETS_MAX {
        return text;
    }
    let union = |names: Vec<String>| {
        names
            .iter()
            .map(|name| format!("{name:?}"))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let hooks: Vec<String> = targets.hooks.iter().cloned().collect();
    let names = if hooks.is_empty() {
        String::from("never")
    } else {
        union(hooks)
    };
    let declared = format!(
        "-- The hooks this app declares (`hook = \"…\"`).\n\
         export type HookName = {names}\n\
         export type HookTarget = {}\n",
        union(targets.written())
    );
    text.replacen(TARGET, &declared, 1).replacen(
        SELECTOR,
        "export type Selector = { __zgx_selector: never }\n",
        1,
    )
}

/// The most routes `RouteName` names; past it the route types stay
/// `string`s, as `MessageKey` does, and the `unknown-route` lint checks
/// names instead.
const ROUTES_MAX: usize = 1000;
/// The most routes `navigate` takes an overload for, each typing its route's
/// params: past it a call takes any route name and loose params (a
/// misspelled name still fails; every overload is listed in a failure's
/// message, which is unreadable by then).
const ROUTE_OVERLOADS: usize = 100;

/// A table type's field for `name`: bare where it can be.
fn field(name: &str) -> String {
    let bare = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !KEYWORDS.contains(&name);
    match bare {
        true => name.to_owned(),
        false => format!("[{name:?}]"),
    }
}

/// `RouteName`, `Navigate` and `RouteCases` for the routes an app's
/// routers declare, in place of zgx's `string`s; unchanged for none, a
/// router it can't read, or past `ROUTES_MAX`.
fn route_types(text: String, routes: &crate::routes::Routes) -> String {
    const NAME: &str = "export type RouteName = string\n";
    const NAVIGATE: &str =
        "export type Navigate = (to: string, params: { [string]: any }?) -> ()\n";
    const CASES: &str =
        "export type RouteCases = { [string]: Node | (params: { [string]: string }) -> Node }\n";
    assert!(
        text.contains(NAME) && text.contains(NAVIGATE) && text.contains(CASES),
        "zgx.d.luau declares RouteName, Navigate and RouteCases loose"
    );
    let routes = &routes.routes;
    if routes.is_empty() || routes.len() > ROUTES_MAX {
        return text;
    }
    let table = |params: &[String], value: &str, access: &str| match params.is_empty() {
        true => String::from("{}"),
        false => {
            let fields: Vec<String> = params
                .iter()
                .map(|param| format!("{access}{}: {value}", field(param)))
                .collect();
            format!("{{ {} }}", fields.join(", "))
        }
    };
    let names: Vec<String> = routes.keys().map(|name| format!("{name:?}")).collect();
    let navigate = match routes.len() <= ROUTE_OVERLOADS {
        true => routes
            .iter()
            .map(|(name, params)| match params.is_empty() {
                true => format!("((to: {name:?}, params: {{}}?) -> ())"),
                false => format!(
                    "((to: {name:?}, params: {}) -> ())",
                    // Read only: the new solver holds a table's fields
                    // invariant, and `{ id = 7 }` is no `{ id: string | number }`.
                    table(params, "string | number", "read ")
                ),
            })
            .collect::<Vec<_>>()
            .join(" & "),
        false => String::from("(to: RouteName, params: { [string]: any }?) -> ()"),
    };
    let cases: Vec<String> = routes
        .iter()
        .map(|(name, params)| {
            format!(
                "{}: (Node | (params: {}) -> Node)?",
                field(name),
                table(params, "string", "")
            )
        })
        .collect();
    text.replacen(
        NAME,
        &format!("export type RouteName = {}\n", names.join(" | ")),
        1,
    )
    .replacen(NAVIGATE, &format!("export type Navigate = {navigate}\n"), 1)
    .replacen(
        CASES,
        &format!("export type RouteCases = {{ {} }}\n", cases.join(", ")),
        1,
    )
}

/// zgx's definitions with the tokens an app's scripts (`sources`) declare.
pub fn definitions(sources: &[(PathBuf, String)]) -> String {
    let read: Vec<_> = sources
        .iter()
        .map(|(path, source)| (path, crate::lint::tokenize(source).0))
        .collect();
    let files: Vec<_> = read
        .iter()
        .map(|(path, tokens)| crate::token_lint::File { path, tokens })
        .collect();
    let declared = crate::token_lint::declared(&files);
    let targets = crate::hook_targets::declared(&files);
    let names: Vec<String> = declared
        .names
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    let standard = standard_tokens_type();
    let ours = tokens_type(&names, &declared.open, declared.all_open);
    assert!(
        DEFINITIONS.contains(&standard),
        "zgx.d.luau's `Tokens` follows the schema"
    );
    let mut text = DEFINITIONS.replacen(&standard, &ours, 1);
    // The keys `ui.t` takes: the source locale's, when the app has a
    // catalog luau-lsp can take as one union.
    if let Some(keys) = crate::lint::source_keys(&files)
        .filter(|keys| !keys.is_empty() && keys.len() <= MESSAGE_KEYS)
    {
        let union: Vec<String> = keys.iter().map(|key| format!("{key:?}")).collect();
        text = text.replacen(
            MESSAGE_KEY_PLACEHOLDER,
            &format!("export type MessageKey = {}", union.join(" | ")),
            1,
        );
    }
    if !names.is_empty() {
        let app: Vec<String> = names.iter().map(|name| format!("\"{name}\"")).collect();
        text = text.replacen(
            "get: (name: TokenName | string) -> any,",
            &format!(
                "get: (name: TokenName | {} | string) -> any,",
                app.join(" | ")
            ),
            1,
        );
    }
    let text = hook_types(text, &targets);
    let text = match crate::routes::declared(&files) {
        routes if routes.open => text,
        routes => route_types(text, &routes),
    };
    format!(
        "-- zgx's definitions with this app's own theme tokens (`ui.theme.tokens`)\n\
         -- and the ids and hooks its plugins may target, its message keys and routes.\n\
         -- Written by `zgx types` (and `zgx check`, and the debug host after a\n\
         -- reload that changes the names): don't edit it, and leave it out of\n\
         -- version control. Editors list it in place of zgx.d.luau.\n{text}"
    )
}

/// Write the definitions of the app in `dir` to `types_file(dir)`, only
/// when they changed (editors watching it don't reload for nothing);
/// whether it did.
pub fn write_types(dir: &Path) -> Result<bool, CheckError> {
    let mut sources = Vec::new();
    for file in scripts(dir) {
        let source = std::fs::read_to_string(&file).map_err(CheckError::Io)?;
        sources.push((file, source));
    }
    write_types_of(dir, &sources)
}

/// As `write_types`, from sources already read.
pub fn write_types_of(dir: &Path, sources: &[(PathBuf, String)]) -> Result<bool, CheckError> {
    let text = definitions(sources);
    let path = types_file(dir);
    if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return Ok(false);
    }
    std::fs::create_dir_all(path.parent().expect("a directory")).map_err(CheckError::Io)?;
    std::fs::write(&path, text).map_err(CheckError::Io)?;
    Ok(true)
}

/// For a plugin's folder (`app_config::lent_to_folder`), the aliases of
/// the packages lent to it added to its `.luaurc` (made if it has none),
/// each naming the folder the app's `zgx.json` gives, from the `.luaurc`:
/// an editor's luau-lsp then resolves and completes `@name/…` as zgx does.
/// An alias already there is left as it is, and so is a `.luaurc` that
/// isn't JSON. The names added; none for a folder that isn't a plugin's.
pub fn write_lent_aliases(dir: &Path) -> Result<Vec<String>, CheckError> {
    let Some(lent) = crate::app_config::lent_to_folder(dir) else {
        return Ok(Vec::new());
    };
    let path = dir.join(".luaurc");
    let mut config: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(config @ serde_json::Value::Object(_)) => config,
            _ => return Ok(Vec::new()),
        },
        Err(_) => serde_json::json!({}),
    };
    let here = dir.canonicalize().map_err(CheckError::Io)?;
    let object = config.as_object_mut().expect("an object");
    let aliases = object
        .entry("aliases")
        .or_insert_with(|| serde_json::json!({}));
    let Some(aliases) = aliases.as_object_mut() else {
        return Ok(Vec::new());
    };
    let mut added = Vec::new();
    for (name, folder) in lent {
        if aliases.contains_key(&name) {
            continue;
        }
        aliases.insert(name.clone(), relative_to(&here, &folder).into());
        added.push(name);
    }
    if !added.is_empty() {
        let text = serde_json::to_string_pretty(&config).expect("JSON") + "\n";
        std::fs::write(&path, text).map_err(CheckError::Io)?;
    }
    Ok(added)
}

/// `to` from `from` (both absolute), with `/` on every system, as a
/// `.luaurc`'s paths are: `../shared/ds`.
fn relative_to(from: &Path, to: &Path) -> String {
    let (from, to): (Vec<_>, Vec<_>) = (from.components().collect(), to.components().collect());
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".into(); from.len() - common];
    parts.extend(
        to[common..]
            .iter()
            .map(|p| p.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(source: &str) -> String {
        definitions(&[(PathBuf::from("app.luau"), source.into())])
    }

    /// An app lending `@ds` (not `@kit`) to its plugin `p`.
    fn lending_app(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("zgx-lent-types-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["app/plugins/p", "shared/ds", "shared/kit"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("app/app.luau"), "return ui.text { \"x\" }").unwrap();
        std::fs::write(root.join("app/plugins/p/init.luau"), "local x = 1").unwrap();
        std::fs::write(
            root.join("app/zgx.json"),
            r#"{ "packages": { "ds": "../shared/ds", "kit": "../shared/kit" },
                 "plugins": { "p": { "path": "plugins/p", "allow": ["package:ds"] } } }"#,
        )
        .unwrap();
        root
    }

    #[test]
    fn a_plugins_lent_packages_are_added_to_its_luaurc_once() {
        let root = lending_app("luaurc");
        let plugin = root.join("app/plugins/p");
        assert_eq!(write_lent_aliases(&plugin).unwrap(), ["ds"]);
        let luaurc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(plugin.join(".luaurc")).unwrap())
                .unwrap();
        // From the .luaurc's folder to the one zgx.json names; kit isn't lent.
        assert_eq!(
            luaurc["aliases"],
            serde_json::json!({ "ds": "../../../shared/ds" })
        );
        assert!(
            write_lent_aliases(&plugin).unwrap().is_empty(),
            "added once"
        );
        // One already there is the author's.
        std::fs::write(
            plugin.join(".luaurc"),
            r#"{ "aliases": { "ds": "./mine" } }"#,
        )
        .unwrap();
        assert!(write_lent_aliases(&plugin).unwrap().is_empty());
        assert!(
            std::fs::read_to_string(plugin.join(".luaurc"))
                .unwrap()
                .contains("./mine")
        );
        // The app's folder is no plugin's: nothing written.
        assert!(write_lent_aliases(&root.join("app")).unwrap().is_empty());
        assert!(!root.join("app/.luaurc").exists());
    }

    #[test]
    fn checking_gives_a_plugin_its_lent_packages_and_the_app_all() {
        let root = lending_app("check");
        let names = |file: &str| -> Vec<String> {
            crate::app_config::aliases_for(&root.join(file))
                .into_iter()
                .map(|(name, _)| name)
                .collect()
        };
        assert_eq!(names("app/plugins/p/init.luau"), ["ds"]);
        assert_eq!(names("app/app.luau"), ["ds", "kit"]);
    }

    #[test]
    fn zgxs_definitions_have_the_message_key_placeholder() {
        assert!(DEFINITIONS.contains(MESSAGE_KEY_PLACEHOLDER));
    }

    #[test]
    fn an_apps_message_keys_are_its_source_locales() {
        let dir = std::env::temp_dir().join(format!("zgx-message-keys-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("lang")).unwrap();
        std::fs::write(
            dir.join("lang/de.json"),
            r#"{ "inbox": { "count": "{n} Nachrichten" }, "save": "Sichern" }"#,
        )
        .unwrap();
        let types = definitions(&[(
            dir.join("app.luau"),
            "ui.i18n { dir = \"lang\", source = \"de\" }".into(),
        )]);
        assert!(
            types.contains("export type MessageKey = \"inbox.count\" | \"save\""),
            "{types}"
        );
        // An app with no catalog keeps `string`.
        assert!(app("return ui.text 'x'").contains(MESSAGE_KEY_PLACEHOLDER));
    }

    #[test]
    fn zgxs_definitions_have_the_standard_tokens_as_fields() {
        assert!(
            DEFINITIONS.contains(&standard_tokens_type()),
            "zgx.d.luau's `Tokens` is not the schema's: write it from `standard_tokens_type`"
        );
    }

    #[test]
    fn a_token_named_by_a_keyword_is_a_quoted_field() {
        let types = app(r##"ui.theme.tokens { fade = { ["end"] = "#000", start = "#fff" } }"##);
        assert!(
            types.contains("\tfade: {\n\t\t[\"end\"]: string,\n\t\tstart: string,\n\t},\n"),
            "{types}"
        );
    }

    #[test]
    fn an_apps_tokens_join_zgxs_as_fields() {
        let types = app(
            r##"ui.theme.tokens { notes = { wash = "#20242c", card = { value = "#fff" } }, ["app.gap"] = 12 }"##,
        );
        assert!(
            types.contains(
                "\tnotes: {\n\t\tcard: {\n\t\t\tvalue: string,\n\t\t},\n\t\twash: string,\n\t},\n"
            ),
            "{types}"
        );
        assert!(
            types.contains("\tapp: {\n\t\tgap: string,\n\t},\n"),
            "{types}"
        );
        assert!(types.contains("\tsurface: {\n"), "zgx's stay: {types}");
        assert!(types.contains("get: (name: TokenName | \"app.gap\" | \"notes.card.value\" | \"notes.wash\" | string)"), "{types}");
    }

    #[test]
    fn a_group_built_at_run_time_takes_any_field() {
        let types = app(r##"ui.theme.tokens { chart = palette() }"##);
        assert!(
            types.contains("\tchart: {\n\t\tvalue: string,\n\t\t[string]: any,\n\t},\n"),
            "{types}"
        );
    }

    #[test]
    fn an_apps_ids_and_hooks_are_the_targets_plugins_may_name() {
        let types = app(
            r#"return ui.row { id = "toolbar", ui.button { id = "save", hook = "toolbar.save", label = "Save" } }"#,
        );
        assert!(
            types.contains("export type HookName = \"toolbar.save\"\n"),
            "{types}"
        );
        assert!(
            types.contains(
                "export type HookTarget = \"#save\" | \"#toolbar\" | \"hook:toolbar.save\"\n"
            ),
            "{types}"
        );
        assert!(
            types.contains("export type Selector = { __zgx_selector: never }\n"),
            "{types}"
        );
        // None declared: zgx's strings, as before.
        let none = app("return ui.text { \"plain\" }");
        assert!(none.contains("export type HookTarget = string\n"));
        assert!(none.contains("export type Selector = string\n"));
    }

    #[test]
    fn an_apps_classes_are_targets_too() {
        let types = app(r#"return ui.row { id = "bar", class = "tool primary", ui.text { "x" } }"#);
        assert!(
            types.contains("export type HookTarget = \"#bar\" | \".primary\" | \".tool\"\n"),
            "{types}"
        );
        // Chains combine freely: any text, which the lint checks.
        assert!(
            types.contains("export type HookChain = string\n"),
            "{types}"
        );
    }

    #[test]
    fn past_the_union_limit_targets_stay_strings() {
        let ids: Vec<String> = (0..=TARGETS_MAX)
            .map(|i| format!("id = \"n{i}\""))
            .collect();
        let types = app(&format!("return ui.column {{ {} }}", ids.join(", ")));
        assert!(types.contains("export type HookTarget = string\n"));
    }

    #[test]
    fn an_apps_routes_type_navigate_and_the_views_cases() {
        let types = app(
            r#"local router = ui.router { routes = { list = "/", thread = "/thread/:id", ["end"] = "/end/:a/:b" } }"#,
        );
        assert!(types.contains("export type RouteName = \"end\" | \"list\" | \"thread\"\n"));
        assert!(types.contains(
            "export type Navigate = ((to: \"end\", params: { read a: string | number, read b: string | number }) -> ()) \
             & ((to: \"list\", params: {}?) -> ()) \
             & ((to: \"thread\", params: { read id: string | number }) -> ())\n"
        ));
        assert!(types.contains(
            "export type RouteCases = { [\"end\"]: (Node | (params: { a: string, b: string }) -> Node)?, \
             list: (Node | (params: {}) -> Node)?, thread: (Node | (params: { id: string }) -> Node)? }\n"
        ));
    }

    #[test]
    fn routes_read_at_run_time_or_past_the_limit_stay_strings() {
        let loose = app("return ui.text { \"plain\" }");
        assert!(loose.contains("export type RouteName = string\n"));
        let open =
            app("local routes = { list = \"/\" }\nlocal router = ui.router { routes = routes }");
        assert!(
            open.contains("export type RouteName = string\n"),
            "routes not literal"
        );
        let many: Vec<String> = (0..=ROUTES_MAX)
            .map(|i| format!("r{i} = \"/r{i}\""))
            .collect();
        let past = app(&format!(
            "local router = ui.router {{ routes = {{ {} }} }}",
            many.join(", ")
        ));
        assert!(past.contains("export type RouteName = string\n"));
        // Past the overloads, a name still checks and params go loose.
        let some: Vec<String> = (0..=ROUTE_OVERLOADS)
            .map(|i| format!("r{i} = \"/r{i}\""))
            .collect();
        let named = app(&format!(
            "local router = ui.router {{ routes = {{ {} }} }}",
            some.join(", ")
        ));
        assert!(named.contains(
            "export type Navigate = (to: RouteName, params: { [string]: any }?) -> ()\n"
        ));
    }

    #[test]
    fn the_file_is_written_only_when_it_changes() {
        let dir = std::env::temp_dir().join(format!("zgx-types-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("app.luau"),
            r##"ui.theme.tokens { app = { card = "#fff" } }"##,
        )
        .unwrap();
        assert!(write_types(&dir).unwrap());
        assert!(
            !write_types(&dir).unwrap(),
            "the same names: left as it was"
        );
        std::fs::write(
            dir.join("app.luau"),
            r##"ui.theme.tokens { app = { card = "#fff", pane = "#000" } }"##,
        )
        .unwrap();
        assert!(write_types(&dir).unwrap());
        assert!(
            std::fs::read_to_string(types_file(&dir))
                .unwrap()
                .contains("pane: string")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
