//! A table benchmark in the spirit of js-framework-benchmark: create 1,000
//! and 10,000 rows, replace them, update every tenth, select one, swap two,
//! remove one, append 1,000 and clear, all through one zgx view. Build it
//! for the browser with `crates/zgx-dom/bench/build.sh`; `bench/run.mjs`
//! times it in headless Chrome.
#![cfg(target_arch = "wasm32")]

use std::{
    cell::{Cell, RefCell},
    hash::{Hash, Hasher},
    rc::Rc,
};
use wasm_bindgen::prelude::*;
use zgx::{
    compose::{View, button, column, keyed, row, switch, text, text_signal, virtual_list},
    reactive::{Runtime, Signal},
    scene::Color,
    style::{Styled, Styles},
};

const FIRST: [&str; 12] = [
    "quiet", "bright", "rapid", "gentle", "bold", "tiny", "vast", "brave", "calm", "eager",
    "fancy", "jolly",
];
const SECOND: [&str; 10] = [
    "amber", "teal", "crimson", "ivory", "olive", "violet", "cobalt", "coral", "slate", "gold",
];
const THIRD: [&str; 11] = [
    "kettle", "lantern", "comet", "harbor", "meadow", "pebble", "rocket", "willow", "anchor",
    "falcon", "garden",
];

struct Item {
    id: usize,
    label: Signal<String>,
    selected: Signal<bool>,
}

/// A row's key: its item, compared by id.
#[derive(Clone)]
struct Row(Rc<Item>);
impl PartialEq for Row {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}
impl Eq for Row {}
impl Hash for Row {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.id.hash(state);
    }
}

struct App {
    runtime: Runtime,
    rows: Signal<Vec<Row>>,
    next: Cell<usize>,
    seed: Cell<u32>,
    selected: RefCell<Option<Rc<Item>>>,
    /// Whether the rows show through a virtual list (only those in view
    /// mounted) rather than all of them.
    virtual_rows: Signal<bool>,
}

impl App {
    fn random(&self, n: usize) -> usize {
        // A fixed sequence, so every run builds the same labels.
        let x = self
            .seed
            .get()
            .wrapping_mul(1_103_515_245)
            .wrapping_add(12_345);
        self.seed.set(x);
        (x >> 16) as usize % n
    }
    fn build(&self, count: usize) -> Vec<Row> {
        (0..count)
            .map(|_| {
                let id = self.next.get();
                self.next.set(id + 1);
                let label = format!(
                    "{} {} {}",
                    FIRST[self.random(FIRST.len())],
                    SECOND[self.random(SECOND.len())],
                    THIRD[self.random(THIRD.len())]
                );
                Row(Rc::new(Item {
                    id,
                    label: self.runtime.signal(label),
                    selected: self.runtime.signal(false),
                }))
            })
            .collect()
    }
    fn run(&self, count: usize) {
        *self.selected.borrow_mut() = None;
        self.virtual_rows.set(false);
        self.rows.set(self.build(count));
    }
    fn run_virtual(&self) {
        *self.selected.borrow_mut() = None;
        self.virtual_rows.set(true);
        self.rows.set(self.build(10_000));
    }
    fn add(&self) {
        let more = self.build(1000);
        self.rows.update(|rows| rows.extend(more));
    }
    fn update(&self) {
        self.rows.with(|rows| {
            for row in rows.iter().step_by(10) {
                row.0.label.update(|label| label.push_str(" !!!"));
            }
        });
    }
    fn clear(&self) {
        *self.selected.borrow_mut() = None;
        self.rows.set(Vec::new());
    }
    fn swap(&self) {
        self.rows.update(|rows| {
            if rows.len() > 998 {
                rows.swap(1, 998);
            }
        });
    }
    fn select(&self, item: &Rc<Item>) {
        if let Some(old) = self.selected.borrow_mut().replace(item.clone()) {
            old.selected.set(false);
        }
        item.selected.set(true);
    }
    fn remove(&self, id: usize) {
        self.rows.update(|rows| rows.retain(|row| row.0.id != id));
    }
}

fn action(id: &str, label: &str, app: &Rc<App>, f: fn(&App)) -> View {
    let app = app.clone();
    button()
        .id(id)
        .px(10.)
        .py(6.)
        .bg(Color(40, 90, 160, 255))
        .rounded(4.)
        .hover(|s| s.bg(Color(50, 105, 185, 255)))
        .child(text(label))
        .on_click(move || f(&app))
}

fn row_view(app: &Rc<App>, item: Rc<Item>) -> View {
    let (select, remove) = (app.clone(), app.clone());
    let (label, selected) = (item.label.clone(), item.selected.clone());
    let id = item.id;
    row()
        .h(28.)
        .items_center()
        .border_edges(zgx::scene::Insets {
            top: 0.,
            right: 0.,
            bottom: 1.,
            left: 0.,
        })
        .border_color(Color(60, 60, 70, 255))
        .reactive_style(move || {
            if selected.get() {
                Styles::new().bg(Color(120, 40, 40, 255))
            } else {
                Styles::new()
            }
        })
        .child(text(id.to_string()).w(80.))
        .child(
            text_signal(move || label.get())
                .grow()
                .on_click(move || select.select(&item)),
        )
        .child(text("x").w(32.).on_click(move || remove.remove(id)))
}

#[wasm_bindgen(start)]
pub fn start() {
    let runtime = Runtime::new();
    let app = Rc::new(App {
        runtime: runtime.clone(),
        rows: runtime.signal(Vec::new()),
        next: Cell::new(1),
        seed: Cell::new(42),
        selected: RefCell::new(None),
        virtual_rows: runtime.signal(false),
    });
    let rows = app.rows.clone();
    let row_app = app.clone();
    let view = column()
        .p(12.)
        .gap(12.)
        .child(row().gap(6.).children([
            action("run", "Create 1,000 rows", &app, |a| a.run(1000)),
            action("runlots", "Create 10,000 rows", &app, |a| a.run(10_000)),
            action("add", "Append 1,000 rows", &app, App::add),
            action("update", "Update every 10th row", &app, App::update),
            action("clear", "Clear", &app, App::clear),
            action("swaprows", "Swap rows", &app, App::swap),
            action(
                "runvirtual",
                "10,000 rows, virtualized",
                &app,
                App::run_virtual,
            ),
        ]))
        .child(switch(
            {
                let mode = app.virtual_rows.clone();
                move || mode.get()
            },
            move |virtualized, cx| {
                let (rows, app) = (rows.clone(), row_app.clone());
                if virtualized {
                    // Only the rows in view (and a few around) mounted.
                    let (count, key) = (rows.clone(), rows.clone());
                    virtual_list(
                        cx.state(0_f32),
                        28.,
                        4,
                        move || count.with(Vec::len),
                        move |index| key.with(|rows| rows[index].clone()),
                        move |_, row: Row, _| row_view(&app, row.0),
                    )
                    .h(600.)
                    .id("vrows")
                } else {
                    keyed(move || rows.get(), move |row: Row, _| row_view(&app, row.0)).id("rows")
                }
            },
        ));
    let mounted = zgx_dom::mount_in_body(&runtime, view);
    // Kept for the page's life.
    std::mem::forget(mounted);
    std::mem::forget(app);
}
