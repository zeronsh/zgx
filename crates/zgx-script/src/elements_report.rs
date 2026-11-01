//! `ScriptHost::elements`: where a plugin can put something and what it can
//! read there. The app's named elements (ids, classes, hooks) grouped by the
//! elements that `provide` context, outermost the app's root, each group
//! with a selector that reaches it and the names its subtree reads. Rows a
//! list repeats are one entry with a count, a private element is listed
//! with nothing inside, and names the source declares that aren't mounted
//! (a page not open) are listed `mounted: false`.
use std::{collections::HashMap, collections::HashSet, rc::Rc};

use zgx::{compose::ViewHandle, scene::NodeId, widgets::Ui};

use crate::{node::Node, value::Value};

/// The app's elements, by the context they read (`ScriptHost::elements`).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ElementsReport {
    /// Whether the mounted interface is in it: kept only while plugins are
    /// loaded (otherwise it costs the app nothing, and lists the source).
    pub live: bool,
    /// The app's root, the outermost group.
    pub root: ElementGroup,
}

/// An element that provides context (`provide`), and what is inside it down
/// to the next.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ElementGroup {
    /// A selector that reaches it: its id, else its classes inside the
    /// nearest element with an id (`#sidebar >> .session-row`). Empty when
    /// nothing names it.
    pub selector: String,
    /// The names it provides.
    pub provides: Vec<String>,
    /// The names provided above it, which its subtree reads too.
    pub inherited: Vec<String>,
    /// Its named elements, down to the next group.
    pub elements: Vec<ListedElement>,
    /// The groups inside it.
    pub groups: Vec<ElementGroup>,
    /// How many alike there are (a list's rows): 1 for one.
    pub count: usize,
}

/// A named element: its id, classes and hook.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ListedElement {
    /// Its id.
    pub id: Option<String>,
    /// Its classes.
    pub classes: Vec<String>,
    /// The hook it declares.
    pub hook: Option<String>,
    /// Out of every plugin's reach: nothing inside it is listed.
    pub private: bool,
    /// Whether it is mounted now; `false`: declared in the source, not
    /// showing (a closed page).
    pub mounted: bool,
    /// How many alike there are.
    pub count: usize,
}

/// The report for `root`, the app's tree, mounted as `mounted` says under
/// `handle`, with what `sources` declare.
pub(crate) fn report(
    ui: &Ui,
    handle: &ViewHandle,
    root: &Node,
    mounted: Option<&HashMap<NodeId, Rc<Node>>>,
    sources: &[(std::path::PathBuf, String)],
) -> ElementsReport {
    let mut group = ElementGroup {
        selector: part(root),
        provides: provided(root),
        inherited: Vec::new(),
        elements: Vec::new(),
        groups: Vec::new(),
        count: 1,
    };
    if let Some(mounted) = mounted {
        let scene = ui.scene.borrow();
        let mut path = Vec::new();
        walk(&scene, handle.node(), mounted, &mut group, &mut path);
    }
    let mut seen_ids = HashSet::new();
    let mut seen_classes = HashSet::new();
    let mut seen_hooks = HashSet::new();
    seen(&group, &mut seen_ids, &mut seen_classes, &mut seen_hooks);
    // Nothing a private element holds is listed, from the source either.
    let mut held = vec![root];
    while let Some(node) = held.pop() {
        if node.private {
            hidden(node, &mut seen_ids, &mut seen_classes, &mut seen_hooks);
            continue;
        }
        held.extend(node.children.iter().map(|c| &**c));
    }
    if let Some(mounted) = mounted {
        for node in mounted.values().filter(|n| n.private) {
            hidden(node, &mut seen_ids, &mut seen_classes, &mut seen_hooks);
        }
    }
    // What the source names that isn't showing, for a page not open.
    let files: Vec<_> = sources
        .iter()
        .map(|(path, source)| (path, crate::lint::tokenize(source).0))
        .collect();
    let files: Vec<_> = files
        .iter()
        .map(|(path, tokens)| crate::token_lint::File { path, tokens })
        .collect();
    let declared = crate::hook_targets::declared(&files);
    let unmounted =
        |id: Option<&String>, class: Option<&String>, hook: Option<&String>| ListedElement {
            id: id.cloned(),
            classes: class.into_iter().cloned().collect(),
            hook: hook.cloned(),
            private: false,
            mounted: false,
            count: 1,
        };
    for id in declared.ids.iter().filter(|id| !seen_ids.contains(*id)) {
        group.elements.push(unmounted(Some(id), None, None));
    }
    for class in declared
        .classes
        .iter()
        .filter(|c| !seen_classes.contains(*c))
    {
        group.elements.push(unmounted(None, Some(class), None));
    }
    for hook in declared.hooks.iter().filter(|h| !seen_hooks.contains(*h)) {
        group.elements.push(unmounted(None, None, Some(hook)));
    }
    collapse(&mut group);
    ElementsReport {
        live: mounted.is_some(),
        root: group,
    }
}

/// `node`'s mounted subtree, into `group`: `path` the listed elements above.
fn walk(
    scene: &zgx::scene::Scene,
    node: NodeId,
    mounted: &HashMap<NodeId, Rc<Node>>,
    group: &mut ElementGroup,
    path: &mut Vec<Rc<Node>>,
) {
    for &child in scene.children(node) {
        let Some(listed) = mounted.get(&child) else {
            walk(scene, child, mounted, group, path);
            continue;
        };
        if listed.private {
            group.elements.push(ListedElement {
                private: true,
                ..element(listed)
            });
            continue;
        }
        path.push(listed.clone());
        if listed.provide.is_some() {
            let mut inherited = group.inherited.clone();
            inherited.extend(group.provides.iter().cloned());
            inherited.sort();
            inherited.dedup();
            let mut inner = ElementGroup {
                selector: selector(path),
                provides: provided(listed),
                inherited,
                elements: Vec::new(),
                groups: Vec::new(),
                count: 1,
            };
            if listed.id.is_some() || !listed.classes.is_empty() || listed.hook().is_some() {
                inner.elements.push(element(listed));
            }
            walk(scene, child, mounted, &mut inner, path);
            group.groups.push(inner);
        } else {
            group.elements.push(element(listed));
            walk(scene, child, mounted, group, path);
        }
        path.pop();
    }
}

fn element(node: &Node) -> ListedElement {
    ListedElement {
        id: node.id.as_deref().map(str::to_owned),
        classes: node.classes.iter().map(|c| c.to_string()).collect(),
        hook: node.hook().map(str::to_owned),
        private: node.private,
        mounted: true,
        count: 1,
    }
}

fn provided(node: &Node) -> Vec<String> {
    let mut names: Vec<String> = node
        .provide
        .iter()
        .flat_map(|given| given.iter().map(|(name, _)| name.to_string()))
        .collect();
    names.sort();
    names
}

/// One element's part of a selector: its id, else its classes, else its
/// hook; empty with none.
fn part(node: &Node) -> String {
    if let Some(id) = &node.id {
        return format!("#{id}");
    }
    if !node.classes.is_empty() {
        return node.classes.iter().map(|c| format!(".{c}")).collect();
    }
    node.hook()
        .map_or(String::new(), |hook| format!("hook:{hook}"))
}

/// A selector for the last of `path`: from the nearest element with an id
/// down to it, each named part joined by `>>`.
fn selector(path: &[Rc<Node>]) -> String {
    let from = path.iter().rposition(|n| n.id.is_some()).unwrap_or(0);
    let parts: Vec<String> = path[from..]
        .iter()
        .map(|n| part(n))
        .filter(|p| !p.is_empty())
        .collect();
    if part(path.last().expect("a path")).is_empty() {
        return String::new();
    }
    parts.join(" >> ")
}

/// What `node`, a private element, holds as written: never listed.
fn hidden(
    node: &Node,
    ids: &mut HashSet<String>,
    classes: &mut HashSet<String>,
    hooks: &mut HashSet<String>,
) {
    ids.extend(node.id.as_deref().map(str::to_owned));
    classes.extend(node.classes.iter().map(|c| c.to_string()));
    hooks.extend(node.hook().map(str::to_owned));
    for child in &node.children {
        hidden(child, ids, classes, hooks);
    }
}
fn seen(
    group: &ElementGroup,
    ids: &mut HashSet<String>,
    classes: &mut HashSet<String>,
    hooks: &mut HashSet<String>,
) {
    for element in &group.elements {
        ids.extend(element.id.clone());
        classes.extend(element.classes.iter().cloned());
        hooks.extend(element.hook.clone());
    }
    for inner in &group.groups {
        seen(inner, ids, classes, hooks);
    }
}

/// Alike elements and alike groups (a list's rows) as one, counted.
fn collapse(group: &mut ElementGroup) {
    let mut elements: Vec<ListedElement> = Vec::new();
    for element in group.elements.drain(..) {
        match elements.iter_mut().find(|e| {
            ListedElement {
                count: e.count,
                ..element.clone()
            } == **e
        }) {
            Some(alike) => alike.count += element.count,
            None => elements.push(element),
        }
    }
    group.elements = elements;
    let mut groups: Vec<ElementGroup> = Vec::new();
    for mut inner in group.groups.drain(..) {
        collapse(&mut inner);
        match groups.iter_mut().find(|g| {
            ElementGroup {
                count: g.count,
                ..inner.clone()
            } == **g
        }) {
            Some(alike) => alike.count += inner.count,
            None => groups.push(inner),
        }
    }
    group.groups = groups;
}

impl ElementsReport {
    /// As JSON (`zgx inspect elements`).
    pub fn to_value(&self) -> Value {
        Value::record([
            ("live", Value::Bool(self.live)),
            ("root", self.root.to_value()),
        ])
    }
    /// A line an element, indented by group, for people.
    pub fn to_text(&self) -> String {
        let mut out = Vec::new();
        self.root.lines(0, &mut out);
        if !self.live {
            out.insert(
                0,
                "(no plugins loaded: the source's names only, nothing mounted is tracked)".into(),
            );
        }
        out.join("\n")
    }
}
impl ElementGroup {
    fn to_value(&self) -> Value {
        let strings = |list: &[String]| {
            Value::List(list.iter().map(|s| Value::Str(s.as_str().into())).collect())
        };
        Value::record([
            ("selector", Value::Str(self.selector.as_str().into())),
            ("provides", strings(&self.provides)),
            ("inherited", strings(&self.inherited)),
            (
                "elements",
                Value::List(self.elements.iter().map(ListedElement::to_value).collect()),
            ),
            (
                "groups",
                Value::List(self.groups.iter().map(ElementGroup::to_value).collect()),
            ),
            ("count", Value::Num(self.count as f64)),
        ])
    }
    fn lines(&self, depth: usize, out: &mut Vec<String>) {
        let pad = "  ".repeat(depth);
        let name = if self.selector.is_empty() {
            "(unnamed)"
        } else {
            &self.selector
        };
        let times = if self.count > 1 {
            format!(" ×{}", self.count)
        } else {
            String::new()
        };
        let mut reads = self.inherited.clone();
        reads.extend(self.provides.iter().cloned());
        out.push(format!(
            "{pad}{name}{times} provides [{}] reads [{}]",
            self.provides.join(", "),
            reads.join(", ")
        ));
        for element in &self.elements {
            let mut what = Vec::new();
            what.extend(element.id.as_ref().map(|id| format!("#{id}")));
            what.extend(element.classes.iter().map(|c| format!(".{c}")));
            what.extend(element.hook.as_ref().map(|h| format!("hook:{h}")));
            let mut notes = Vec::new();
            if element.count > 1 {
                notes.push(format!("×{}", element.count));
            }
            if element.private {
                notes.push("private".into());
            }
            if !element.mounted {
                notes.push("not mounted".into());
            }
            let notes = if notes.is_empty() {
                String::new()
            } else {
                format!(" ({})", notes.join(", "))
            };
            out.push(format!("{pad}  {}{notes}", what.join(" ")));
        }
        for inner in &self.groups {
            inner.lines(depth + 1, out);
        }
    }
}
impl ListedElement {
    fn to_value(&self) -> Value {
        Value::record([
            (
                "id",
                self.id
                    .as_deref()
                    .map_or(Value::Nil, |id| Value::Str(id.into())),
            ),
            (
                "classes",
                Value::List(
                    self.classes
                        .iter()
                        .map(|c| Value::Str(c.as_str().into()))
                        .collect(),
                ),
            ),
            (
                "hook",
                self.hook
                    .as_deref()
                    .map_or(Value::Nil, |h| Value::Str(h.into())),
            ),
            ("private", Value::Bool(self.private)),
            ("mounted", Value::Bool(self.mounted)),
            ("count", Value::Num(self.count as f64)),
        ])
    }
}
