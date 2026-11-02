//! `ui.modifiers`: the modifier keys held now, a modifier alone included,
//! each a tracked state; a script that never reads it makes nothing.
mod common;
use common::Harness;
use zgx::input::{Modifiers, set_held_modifiers};

#[test]
fn command_held_alone_shows_in_what_reads_ui_modifiers_and_clears() {
    let mut h = Harness::inline(
        r#"local held = ui.modifiers
        return ui.text {
            id = "held",
            function()
                return (held.meta:get() and "meta" or "-")
                    .. (held.shift:get() and " shift" or "")
            end,
        }"#,
    );
    assert_eq!(h.error(), None);
    h.settle();
    assert_eq!(h.text("held"), "-");
    let runtime = h.ui.runtime.clone();
    let command = Modifiers {
        meta: true,
        ..Modifiers::default()
    };
    // Command alone, as the host reports it: no key event, the state.
    set_held_modifiers(&runtime, command);
    h.settle();
    assert_eq!(h.text("held"), "meta");
    set_held_modifiers(
        &runtime,
        Modifiers {
            shift: true,
            ..command
        },
    );
    h.settle();
    assert_eq!(h.text("held"), "meta shift");
    // The window loses focus: all up.
    set_held_modifiers(&runtime, Modifiers::default());
    h.settle();
    assert_eq!(h.text("held"), "-");
}

#[test]
fn a_script_not_reading_ui_modifiers_makes_no_state_of_them() {
    let mut h = Harness::inline(
        r#"return ui.text { tostring(rawget(ui, "modifiers") == nil), id = "none" }"#,
    );
    assert_eq!(h.error(), None);
    assert_eq!(h.text("none"), "true");
    let effects = h.ui.runtime.effect_count();
    set_held_modifiers(
        &h.ui.runtime,
        Modifiers {
            meta: true,
            ..Modifiers::default()
        },
    );
    h.settle();
    assert_eq!(h.ui.runtime.effect_count(), effects, "nothing runs");
}
