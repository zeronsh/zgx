//! Animation tracks still at what their property holds (a reorder's rows
//! held across while they move up and down, an animation from a value to
//! itself) skip their frames: what an app shows, frame by frame, is what
//! writing them every frame showed.
mod common;

use common::Harness;

/// Every frame's look of `ids`: where each shows, its node's translation,
/// and whether the clock still wants frames, to the bit.
fn frames(
    code: &str,
    ids: &[&str],
    skip: bool,
    steps: &mut dyn FnMut(&mut Harness, usize),
) -> Vec<String> {
    zgx_script::still_tracks_skip(skip);
    let mut h = Harness::inline(code);
    let mut out = Vec::new();
    for frame in 0..40 {
        steps(&mut h, frame);
        h.frame();
        let mut look = String::new();
        for id in ids {
            match h.find(id) {
                Some(node) => {
                    let scene = h.ui.scene.borrow();
                    let shown = scene.transformed_bounds(node);
                    let moved = scene.transform(node);
                    look += &format!(
                        "{id} {:?} {:?} ",
                        [shown.x, shown.y, shown.width, shown.height].map(f32::to_bits),
                        [moved.x, moved.y].map(f32::to_bits)
                    );
                }
                None => look += &format!("{id} gone "),
            }
        }
        look += &format!("wants {}", h.clock.wants_frame());
        out.push(look);
    }
    zgx_script::still_tracks_skip(true);
    out
}

/// `code` driven by `steps` shows the same with still tracks skipping their
/// frames as writing them, and at least `least` frames were skipped.
fn same(code: &str, ids: &[&str], least: u64, mut steps: impl FnMut(&mut Harness, usize)) {
    let before = zgx_script::still_tracks_skipped();
    let skipped = frames(code, ids, true, &mut steps);
    let skips = zgx_script::still_tracks_skipped() - before;
    assert!(skips >= least, "still tracks skipped {skips} frames");
    let written = frames(code, ids, false, &mut steps);
    assert_eq!(
        zgx_script::still_tracks_skipped() - before,
        skips,
        "none skip when off"
    );
    for (n, (a, b)) in skipped.iter().zip(&written).enumerate() {
        assert_eq!(a, b, "frame {n}");
    }
}

/// Rows of a list reordered by `#flip`, `#drop_b` and `#add_d`, in a column
/// (moving up and down only) or a grid (across too).
fn list(container: &str) -> String {
    format!(
        r##"local rows = ui.state("rows", {{ {{ id = "a" }}, {{ id = "b" }}, {{ id = "c" }}, {{ id = "e" }} }})
        ui.command {{ id = "flip", run = function()
            local out = {{}}
            for _, r in rows:get() do table.insert(out, 1, r) end
            rows:set(out)
        end }}
        ui.command {{ id = "drop_b", run = function()
            local out = {{}}
            for _, r in rows:get() do if r.id ~= "b" then table.insert(out, r) end end
            rows:set(out)
        end }}
        ui.command {{ id = "add_d", run = function()
            local out = {{ {{ id = "d" }} }}
            for _, r in rows:get() do table.insert(out, r) end
            rows:set(out)
        end }}
        return ui.column {{
            ui.each(rows, function(r)
                return ui.div {{ id = r.id, w = 30, h = 10, shrink = 0 }}
            end, {{ key = function(r) return r.id end,
                   animate = {{ duration = 200, ease = "linear" }}, {container} }}),
        }}"##
    )
}

#[test]
fn a_list_reordered_up_and_down_shows_what_writing_every_frame_showed() {
    let run = |h: &mut Harness, frame: usize| match frame {
        2 => assert!(h.host.run_command("flip")),
        // Mid-move: rows in flight are held from where they show.
        6 => assert!(h.host.run_command("flip")),
        // A row leaves and one arrives while the rest still move.
        9 => assert!(h.host.run_command("drop_b")),
        12 => assert!(h.host.run_command("add_d")),
        _ => {}
    };
    same(&list(""), &["a", "b", "c", "d", "e"], 20, run);
}

#[test]
fn a_grid_reordered_across_mid_move_shows_what_writing_every_frame_showed() {
    // In a grid of two columns, a flip moves some rows across and down and
    // others down alone; a second flip mid-move holds rows in flight across
    // from where they show (not still), and a removal holds again.
    let run = |h: &mut Harness, frame: usize| match frame {
        2 => assert!(h.host.run_command("flip")),
        5 => assert!(h.host.run_command("flip")),
        7 => assert!(h.host.run_command("drop_b")),
        10 => assert!(h.host.run_command("flip")),
        _ => {}
    };
    // (Every row moves across in this one: what it checks is that rows in
    // flight across are never taken for still.)
    same(
        &list("grid = true, cols = 2, w = 80"),
        &["a", "b", "c", "d", "e"],
        0,
        run,
    );
}

#[test]
fn a_still_track_after_another_writer_shows_what_writing_every_frame_showed() {
    // An animation that takes no ownership (`composition = "none"`) moves
    // `#box` every frame, and a still one, started after it, writes its value
    // after it each frame: writing every frame, the box shows the still
    // value. The still track's property then never holds its value when its
    // turn comes, so it never skips: the box shows what it showed.
    let code = r##"ui.command { id = "chase", run = function()
            anime.animate("#box", { x = { 0, 40 }, composition = "none", duration = 400, ease = "linear" })
        end }
        ui.command { id = "hold", run = function()
            anime.animate("#box", { x = { 5, 5 }, duration = 300, ease = "linear" })
        end }
        return ui.column { ui.div { id = "box", w = 10, h = 10 } }"##;
    let run = |h: &mut Harness, frame: usize| match frame {
        2 => assert!(h.host.run_command("chase")),
        4 => assert!(h.host.run_command("hold")),
        _ => {}
    };
    same(code, &["box"], 0, run);
}

#[test]
fn an_animation_from_a_value_to_itself_shows_what_writing_every_frame_showed() {
    // `#idle` is animated nowhere else: its still track is all that writes
    // it, so skipping the frames leaves it out of the flush altogether. A
    // later animation of the same property, delayed, waits on the still one
    // until its tween starts, then takes the property from it.
    let code = r##"ui.command { id = "hold", run = function()
            anime.animate("#idle", { x = { 5, 5 }, duration = 300, ease = "linear" })
            anime.animate("#moving", { x = { 5, 5 }, y = { 0, 40 }, duration = 300, ease = "linear" })
        end }
        ui.command { id = "later", run = function()
            anime.animate("#idle", { x = { 5, 25 }, delay = 100, duration = 100, ease = "linear" })
        end }
        return ui.column {
            ui.div { id = "idle", w = 10, h = 10 },
            ui.div { id = "moving", w = 10, h = 10 },
        }"##;
    let run = |h: &mut Harness, frame: usize| match frame {
        2 => assert!(h.host.run_command("hold")),
        8 => assert!(h.host.run_command("later")),
        _ => {}
    };
    same(code, &["idle", "moving"], 10, run);
}
