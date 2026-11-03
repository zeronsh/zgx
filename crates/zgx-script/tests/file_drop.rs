//! Files dragged in from outside the window (`on_file_hover`,
//! `on_file_drop`), as react-dropzone hears them: each a `File` with its
//! name, path, size and media type.
mod common;
use common::Harness;
use std::{path::PathBuf, sync::Arc};
use zgx::input::InputEvent;

const APP: &str = r#"
local over = ui.state("over", false)
local dropped = ui.state("dropped", "")
return ui.column {
    p = 10,
    ui.column {
        id = "zone",
        w = 200,
        h = 100,
        on_file_hover = function(now) over:set(now) end,
        on_file_drop = function(files, e)
            local names = {}
            for _, f in files do
                table.insert(names, `{f.name} {f.size} {f.type}`)
            end
            dropped:set(`{table.concat(names, ", ")} at {e.local_x},{e.local_y}`)
        end,
        ui.text { id = "over", function() return if over:get() then "over" else "away" end },
    },
    ui.text { id = "dropped", "{dropped}" },
}
"#;

/// Two files to drop, in a directory of `test`'s own: tests run at once,
/// and one writing the files another is reading showed it a size of 0.
fn files(test: &str) -> Vec<PathBuf> {
    let dir = std::env::temp_dir().join(format!("zgx-file-drop-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let photo = dir.join("photo.PNG");
    std::fs::write(&photo, [0u8; 12]).unwrap();
    let notes = dir.join("notes.unknown");
    std::fs::write(&notes, "hello").unwrap();
    vec![photo, notes]
}

#[test]
fn files_dragged_over_and_dropped_reach_the_zone() {
    let mut h = Harness::inline(APP);
    let paths = files("dropped");
    h.ui.prepare_frame();
    assert_eq!(h.text("over"), "away");
    h.ui.dispatch(InputEvent::FileHover {
        x: 50.,
        y: 40.,
        path: paths[0].clone(),
    });
    h.frames(1);
    assert_eq!(h.text("over"), "over");
    // Moving within it stays over.
    h.ui.dispatch(InputEvent::FileHover {
        x: 60.,
        y: 70.,
        path: paths[0].clone(),
    });
    h.frames(1);
    assert_eq!(h.text("over"), "over");
    h.ui.dispatch(InputEvent::FilesDrop {
        x: 60.,
        y: 70.,
        paths: Arc::from(paths),
    });
    h.frames(1);
    assert_eq!(h.text("over"), "away");
    assert_eq!(
        h.text("dropped"),
        "photo.PNG 12 image/png, notes.unknown 5  at 50,60"
    );
    assert_eq!(h.error(), None);
}

#[test]
fn a_drag_that_leaves_ends_the_hover() {
    let mut h = Harness::inline(APP);
    let paths = files("leaves");
    h.ui.prepare_frame();
    h.ui.dispatch(InputEvent::FileHover {
        x: 50.,
        y: 40.,
        path: paths[0].clone(),
    });
    h.frames(1);
    assert_eq!(h.text("over"), "over");
    h.ui.dispatch(InputEvent::FileHoverCancelled);
    h.frames(1);
    assert_eq!(h.text("over"), "away");
    // Moved out of the zone, onto the text below it: away too.
    h.ui.dispatch(InputEvent::FileHover {
        x: 50.,
        y: 40.,
        path: paths[0].clone(),
    });
    h.frames(1);
    h.ui.dispatch(InputEvent::FileHover {
        x: 300.,
        y: 300.,
        path: paths[0].clone(),
    });
    h.frames(1);
    assert_eq!(h.text("over"), "away");
}

/// What the files were over goes as they hover (a row made again, a hint
/// hidden by the hover itself): the zone round it still hears them leave,
/// and hears them move on to another zone.
const GONE: &str = r#"
local over = ui.state("over", false)
local other = ui.state("other", false)
return ui.column {
    p = 10,
    ui.column {
        id = "zone",
        w = 200,
        h = 100,
        on_file_hover = function(now) over:set(now) end,
        ui.show(function() return not over:get() end, ui.div {
            id = "hint",
            w = 200,
            h = 100,
            on_click = function() end,
        }),
    },
    ui.column {
        id = "beside",
        w = 200,
        h = 100,
        on_file_hover = function(now) other:set(now) end,
    },
    ui.text { id = "over", function() return if over:get() then "over" else "away" end },
    ui.text { id = "other", function() return if other:get() then "over" else "away" end },
}
"#;

#[test]
fn the_zone_hears_the_leave_when_what_the_files_were_over_is_gone() {
    let mut h = Harness::inline(GONE);
    let paths = files("gone");
    h.ui.prepare_frame();
    let hover = |h: &mut Harness, y: f32| {
        h.ui.dispatch(InputEvent::FileHover {
            x: 50.,
            y,
            path: paths[0].clone(),
        });
        h.frames(1);
    };
    // Over the hint, which the hover hides.
    hover(&mut h, 40.);
    assert_eq!(h.text("over"), "over");
    h.ui.dispatch(InputEvent::FileHoverCancelled);
    h.frames(1);
    assert_eq!(h.text("over"), "away");
    // Again, and on to the zone beside it: the first hears them go.
    hover(&mut h, 40.);
    assert_eq!(h.text("over"), "over");
    hover(&mut h, 140.);
    assert_eq!(
        (h.text("over"), h.text("other")),
        ("away".into(), "over".into())
    );
    h.ui.dispatch(InputEvent::FileHoverCancelled);
    h.frames(1);
    assert_eq!(h.text("other"), "away");
    assert_eq!(h.error(), None);
}
