//! A style that can't change (no reactive style, variant, interaction,
//! disabled state, token, typography of its own or backdrop blur; not
//! text) keeps no style effect
//! (`StaticStyles::Once`): it runs once, where its effect's first run
//! would, and keeps nothing, so the node shows and damages what it did;
//! what can change keeps an effect and follows.
use zgx::{
    compose::prelude::*,
    cursor::Cursor,
    scene::{Color, NodeId, NodeKind},
    theme::tok,
    widgets::{StaticStyles, Ui},
};

/// What a node shows of its style.
fn state(ui: &Ui, node: NodeId) -> String {
    let scene = ui.scene.borrow();
    format!(
        "{:?}",
        (
            scene.style(node),
            scene.kind(node),
            scene.effects(node),
            scene.transform(node),
            scene.local_transform(node),
            scene.is_isolated(node),
            scene.mask(node),
            scene.clip_inset(node),
        )
    )
}
/// Every node's, depth first.
fn states(ui: &Ui) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![ui.root()];
    while let Some(node) = stack.pop() {
        out.push(state(ui, node));
        stack.extend(ui.scene.borrow().children(node).iter().rev());
    }
    out
}
fn catalogue(ui: &Ui) -> View {
    let offset = ui.signal(0.);
    column()
        .p(8.)
        .gap(6.)
        .items_center()
        .justify_between()
        .child(
            div()
                .w(120.)
                .h(30.)
                .bg(Color(10, 20, 30, 255))
                .rounded(6.)
                .border(2.),
        )
        .child(
            row()
                .w_full()
                .h(20.)
                .px(4.)
                .gap(2.)
                .child(div().w(10.).h(10.)),
        )
        .child(
            div()
                .w(40.)
                .h(40.)
                .opacity(0.5)
                .rotate(0.2)
                .scale(1.5)
                .translate(3., 4.),
        )
        .child(div().w(50.).h(50.).overflow_hidden().fade_edges(4., 6.))
        .child(div().w(20.).h(20.).cursor(Cursor::Move))
        .child(
            scroll(offset).w(100.).h(60.).child(
                column()
                    .gap(4.)
                    .child(div().w(80.).h(30.).bg(Color(1, 2, 3, 255))),
            ),
        )
        .child(text("words").id("text"))
}

#[test]
fn a_static_style_shows_and_damages_what_its_effect_did() {
    // Mounted; then a reactive node beside the static ones restyled; then
    // their parent's typography and the theme changed under them: each
    // flush's damage and every node's state, under each mode.
    let run = |mode| {
        let mut ui = Ui::new(400., 400.);
        ui.set_static_styles(mode);
        let wide = ui.signal(false);
        let read = wide.clone();
        let size = ui.signal(12.);
        let sized = size.clone();
        let view = column()
            .reactive_style(move || Styles::new().text_size(sized.get()))
            .child(catalogue(&ui))
            .child(div().w(20.).h(20.).bg(tok::ACCENT))
            .child(
                div()
                    .h(10.)
                    .reactive_style(move || Styles::new().w(if read.get() { 300. } else { 100. })),
            );
        ui.mount(view);
        let mut flushes = Vec::new();
        for step in 0..4 {
            if step == 1 {
                wide.set(true);
            }
            if step == 2 {
                size.set(18.);
                ui.theme()
                    .set_system_tokens([(tok::ACCENT.0, Color(200, 10, 10, 255).into())]);
            }
            ui.prepare_frame();
            let flush = ui.scene.borrow_mut().flush();
            flushes.push(format!("{:?}", flush.damage));
        }
        (states(&ui), flushes, ui.runtime.effect_count())
    };
    let (once, once_damage, once_effects) = run(StaticStyles::Once);
    let (effect, effect_damage, effect_effects) = run(StaticStyles::Effect);
    assert_eq!(once, effect, "every node as its effect leaves it");
    assert_eq!(once_damage, effect_damage, "every flush damages the same");
    assert!(
        once_effects < effect_effects,
        "fewer effects kept: {once_effects} against {effect_effects}"
    );
}

#[test]
fn a_static_box_passes_on_typography_that_changes() {
    let mut ui = Ui::new(400., 300.);
    let size = ui.signal(10.);
    let read = size.clone();
    let view = ui.mount(
        column()
            .items_start()
            .reactive_style(move || Styles::new().text_size(read.get()))
            .child(
                div()
                    .bg(Color(9, 9, 9, 255))
                    .p(2.)
                    .child(text("abcd").id("t")),
            ),
    );
    ui.prepare_frame();
    let t = view.find("t").unwrap();
    assert_eq!(ui.scene.borrow().bounds(t).width, 24.);
    size.set(20.);
    ui.prepare_frame();
    assert_eq!(ui.scene.borrow().bounds(t).width, 48., "the text followed");
}

fn fill(ui: &Ui, node: NodeId) -> Color {
    match ui.scene.borrow().kind(node) {
        NodeKind::Panel { quad, .. } => quad.fill,
        other => panic!("not a panel: {other:?}"),
    }
}

#[test]
fn a_tokened_style_follows_the_theme_and_a_literal_one_stays() {
    let mut ui = Ui::new(200., 200.);
    let view = ui.mount(
        column()
            .child(div().w(10.).h(10.).bg(tok::ACCENT).id("tokened"))
            .child(div().w(10.).h(10.).bg(Color(1, 2, 3, 255)).id("literal")),
    );
    ui.prepare_frame();
    let (tokened, literal) = (view.find("tokened").unwrap(), view.find("literal").unwrap());
    let accent = Color(200, 10, 10, 255);
    assert_ne!(fill(&ui, tokened), accent);
    ui.theme()
        .set_system_tokens([(tok::ACCENT.0, accent.into())]);
    ui.prepare_frame();
    assert_eq!(fill(&ui, tokened), accent, "the token's new value");
    assert_eq!(fill(&ui, literal), Color(1, 2, 3, 255), "a literal stays");
}

#[test]
fn frosted_glass_keeps_following_reduced_transparency() {
    let mut ui = Ui::new(200., 200.);
    let view = ui.mount(
        column().child(
            div()
                .w(10.)
                .h(10.)
                .backdrop_blur(4.)
                .bg(Color(0, 0, 0, 0))
                .id("glass"),
        ),
    );
    ui.prepare_frame();
    let glass = view.find("glass").unwrap();
    assert!(ui.scene.borrow().effects(glass).blur_radius > 0.);
    ui.theme()
        .set_system_preferences(zgx::theme::SystemPreferences {
            reduced_transparency: true,
            ..Default::default()
        });
    ui.prepare_frame();
    assert_eq!(
        ui.scene.borrow().effects(glass).blur_radius,
        0.,
        "opaque: it kept its effect"
    );
}

#[test]
fn what_can_change_keeps_its_effect() {
    let mut ui = Ui::new(200., 200.);
    let on = ui.signal(false);
    let read = on.clone();
    let view = ui.mount(
        column()
            .child(div().w(10.).h(10.).id("reactive").reactive_style(move || {
                Styles::new().bg(if read.get() {
                    Color(9, 9, 9, 255)
                } else {
                    Color(1, 1, 1, 255)
                })
            }))
            .child(
                div()
                    .w(10.)
                    .h(10.)
                    .bg(Color(1, 1, 1, 255))
                    .hover(|s| s.bg(Color(7, 7, 7, 255)))
                    .id("hovered"),
            ),
    );
    ui.prepare_frame();
    let reactive = view.find("reactive").unwrap();
    on.set(true);
    ui.prepare_frame();
    assert_eq!(fill(&ui, reactive), Color(9, 9, 9, 255));
    let hovered = view.find("hovered").unwrap();
    let at = ui.scene.borrow().bounds(hovered);
    ui.dispatch(zgx::input::InputEvent::PointerMove {
        x: at.x + 5.,
        y: at.y + 5.,
    });
    ui.prepare_frame();
    assert_eq!(fill(&ui, hovered), Color(7, 7, 7, 255), "hovered");
}
