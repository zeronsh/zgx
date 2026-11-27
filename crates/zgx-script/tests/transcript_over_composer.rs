//! A chat's transcript (`ui.list { measured, follow }`) resting at its end
//! over a composer that grows and shrinks as rows come (`items:push`), with
//! nothing opaque behind the composer: each frame drawn from its damage
//! alone shows what a whole frame does. The view's rows moved as it changed
//! height, and a row's pixels stayed where the view had been.
mod common;
use common::Harness;
use zgx::scene::Rect;
use zgx_gpu::GpuRenderer;

const W: f32 = 320.;
const H: f32 = 400.;
fn app(fill: &str, window: bool) -> String {
    format!(
        r##"
local items = ui.state("items", (function()
    local list = {{}}
    for n = 1, 60 do table.insert(list, {{ id = n }}) end
    return list
end)())
local offset = ui.state("offset", 0)
local composer = ui.state("composer", 60)
local step = ui.state("step", 0)
-- Grow with a row; a row alone; shrink with a row; grow alone; shrink alone;
-- a big grow with two rows; a big shrink; grow with a row.
local moves = {{ {{20, 1}}, {{0, 1}}, {{-20, 1}}, {{20, 0}}, {{-20, 0}}, {{60, 2}}, {{-60, 0}}, {{40, 1}} }}
local colors = {{ "#d04040", "#40a040", "#4060d0", "#c0a030", "#8040a0" }}
return ui.column {{
    w = {W}, h = {H}, {bg}
    ui.list {{
        id = "list", w = {W}, grow = 1, min_h = 0,
        height = 36, measured = true, follow = true,
        data = items, key = "id", offset = offset,
        row = function(item)
            return ui.div {{ w = {W}, h = 36, pt = 4,
                ui.div {{ w = 220, h = 28, bg = colors[(item.id % 5) + 1] }} }}
        end,
    }},
    ui.div {{ id = "composer", w = {W}, h = composer, {fill} }},
    ui.button {{ id = "step", absolute = true, top = 0, right = 0, w = 2, h = 2,
        on_click = function()
            local m = moves[(step:get() % #moves) + 1]
            step:set(step:get() + 1)
            if m[1] ~= 0 then composer:set(composer:get() + m[1]) end
            for _ = 1, m[2] do items:push({{ id = #items:get() + 1 }}) end
        end }},
    ui.button {{ id = "far", absolute = true, top = 0, right = 4, w = 2, h = 2,
        on_click = function() offset:set(100000) end }},
    ui.text {{ id = "offset", absolute = true, top = 0, left = 0, w = 1, h = 1,
        function() return tostring(offset:get()) end }},
}}
"##,
        bg = if window { "bg = \"#101820\"," } else { "" },
        fill = fill,
    )
}
fn run(fill: &str, window: bool, copies: bool, scale: f32) -> (usize, usize, usize, Vec<String>) {
    let mut h = Harness::inline(&app(fill, window));
    h.frames(4);
    // At its end, as a chat rests.
    h.click("far");
    h.frames(4);
    let rested: f32 = h.text("offset").parse().unwrap();
    assert!(rested > 1000., "the list rests at its end: {rested}");
    let px = |v: f32| (v * scale) as u32;
    let mut partial = GpuRenderer::new(px(W), px(H)).unwrap();
    partial.set_scale_factor(scale);
    partial.set_background(zgx::scene::Color(16, 24, 32, 255));
    if copies {
        partial.debug_allow_scroll_copies();
    }
    let mut whole = GpuRenderer::new(px(W), px(H)).unwrap();
    whole.set_scale_factor(scale);
    whole.set_background(zgx::scene::Color(16, 24, 32, 255));
    let full = [Rect::new(0., 0., W, H)];
    h.ui.prepare_frame();
    h.ui.scene.borrow_mut().flush();
    partial.render(&h.ui.scene.borrow(), &full).unwrap();
    partial.readback().unwrap();
    let grows = [true, false, false, true, false, true, false, true];
    let (mut both, mut stale_frames, mut worst, mut notes) = (0, 0, 0, Vec::new());
    for step in 0..24 {
        let was: f32 = h.text("offset").parse().unwrap();
        h.click("step");
        h.ui.prepare_frame();
        let report = h.ui.scene.borrow_mut().flush();
        partial
            .render(&h.ui.scene.borrow(), &report.damage)
            .unwrap();
        let shown = partial.readback().unwrap();
        whole.render(&h.ui.scene.borrow(), &full).unwrap();
        let fresh = whole.readback().unwrap();
        let stale = shown
            .chunks(4)
            .zip(fresh.chunks(4))
            .filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 1))
            .count();
        let now: f32 = h.text("offset").parse().unwrap();
        if now != was && grows[step % 8] {
            both += 1;
        }
        if stale > 0 {
            stale_frames += 1;
            worst = worst.max(stale);
            notes.push(format!("step {step} offset {was}->{now}: {stale}"));
        }
    }
    (both, stale_frames, worst, notes)
}
#[test]
fn a_transcript_over_a_composer_that_changes_height_leaves_no_rows_behind() {
    for (name, fill) in [
        ("none", ""),
        ("translucent", "bg = \"#ffffff5a\","),
        ("opaque", "bg = \"#f0f0f0\","),
    ] {
        for window in [false, true] {
            for copies in [false, true] {
                for scale in [1., 2.] {
                    let (both, stale, _, notes) = run(fill, window, copies, scale);
                    let what = format!(
                        "composer {name}, window background {window}, copies {copies}, {scale}x"
                    );
                    // (Else nothing here was checked.)
                    assert!(
                        both >= 6,
                        "{what}: the list moved as the composer grew in only {both} frames"
                    );
                    assert_eq!(stale, 0, "{what}: {notes:?}");
                }
            }
        }
    }
}
