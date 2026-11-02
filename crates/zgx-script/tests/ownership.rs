//! What owns a cleanup, a timer or an effect a script makes: the innermost
//! of the build it is made in and the effect or computed running. An
//! effect's run owns what it makes until its next run, as a React effect's
//! cleanup runs between runs; content a `ui.presence` swaps out takes its
//! effects, and what they made, with it.
mod common;

use common::Harness;

/// A cleanup made in an effect runs before the effect runs again, and when
/// what holds the effect goes: here a `ui.presence` swapping its content.
#[test]
fn an_effects_cleanup_runs_between_runs_and_when_presence_swaps_it_out() {
    let mut h = Harness::inline(
        r##"local tab = ui.state("tab", "a")
        local tick = ui.state("tick", 0)
        local cleaned = ui.state("cleaned", "")
        return ui.column {
            ui.presence(tab, function(v)
                ui.effect(function()
                    local n = tick:get()
                    ui.on_cleanup(function() cleaned:set(cleaned:peek() .. v .. n .. " ") end)
                end)
                return ui.text { v, id = v }
            end),
            ui.button { id = "tick", w = 10, h = 10, on_click = function() tick:set(tick:peek() + 1) end },
            ui.button { id = "swap", w = 10, h = 10, on_click = ui.set(tab, "b") },
            ui.text { id = "cleaned", "{cleaned}" },
        }"##,
    );
    assert_eq!(h.text("cleaned"), "");
    h.click("tick");
    h.frames(2);
    assert_eq!(
        h.text("cleaned"),
        "a0 ",
        "the last run's cleanup, before the next"
    );
    h.click("tick");
    h.frames(2);
    assert_eq!(h.text("cleaned"), "a0 a1 ");
    h.click("swap");
    h.frames(2);
    assert_eq!(
        h.text("cleaned"),
        "a0 a1 a2 ",
        "the content swapped out takes it"
    );
}

/// A timer made in a computed is its run's: it fires unless the computed
/// runs again first, which stops it and starts its next run's. A computed
/// is a derivation, as React's `useMemo` is; what it starts lasts a run.
#[test]
fn a_timer_made_in_a_computed_lasts_until_its_next_run() {
    let mut h = Harness::inline(
        r##"local input = ui.state("input", 1)
        local fired = ui.state("fired", "")
        local doubled = ui.computed(function()
            local n = input:get()
            ui.after(30, function() fired:set(fired:peek() .. n .. " ") end)
            return n * 2
        end)
        return ui.column {
            ui.text { id = "doubled", function() return tostring(doubled:get()) end },
            ui.button { id = "two", w = 10, h = 10, on_click = ui.set(input, 2) },
            ui.text { id = "fired", "{fired}" },
        }"##,
    );
    // Changed at once: the first run's timer stops, the second's fires.
    h.click("two");
    assert!(h.wait_until(|app| app.text("fired") == "2 "));
    std::thread::sleep(std::time::Duration::from_millis(60));
    h.frames(2);
    assert_eq!(h.text("fired"), "2 ", "the first run's never fired");
}
