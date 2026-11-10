//! A large virtual-list jump must erase the old rows, including pixels
//! now lying in gaps between the replacement rows.
use zgx::{compose::prelude::*, scene::Rect, widgets::Ui};
use zgx_gpu::GpuRenderer;

#[test]
fn a_virtual_list_jump_erases_the_rows_it_unmounted() {
    let (w, h) = (240., 180.);
    for scale in [1., 1.5, 2.] {
        let mut ui = Ui::new(w, h);
        let offset = ui.signal(1000_f32);
        ui.mount(
            virtual_list(
                offset.clone(),
                24.,
                2,
                || 1000,
                |i| i,
                |_, _, _| div().size(120., 14.).bg(rgb(0xffffff)),
            )
            .w(w)
            .h(h),
        );
        let mut partial = GpuRenderer::new((w * scale) as u32, (h * scale) as u32).unwrap();
        partial.set_scale_factor(scale);
        partial.debug_allow_scroll_copies();
        let mut whole = GpuRenderer::new((w * scale) as u32, (h * scale) as u32).unwrap();
        whole.set_scale_factor(scale);
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        let full = [Rect::new(0., 0., w, h)];
        partial.render(&ui.scene.borrow(), &full).unwrap();
        partial.readback().unwrap();
        offset.set(0.);
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        partial.render(&ui.scene.borrow(), &report.damage).unwrap();
        let shown = partial.readback().unwrap();
        whole.render(&ui.scene.borrow(), &full).unwrap();
        let expected = whole.readback().unwrap();
        let stale = shown
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(
            stale, 0,
            "old virtual rows survived a jump at {scale}x: {:?}",
            report.damage
        );
    }
}

#[test]
fn a_row_removed_after_a_noncopying_jump_leaves_no_pixels() {
    removed_after_move(-400., false, false, Change::Remove);
}

#[test]
fn a_row_removed_after_an_isolated_move_damages_its_old_pixels() {
    removed_after_move(-16., true, false, Change::Remove);
}

#[test]
fn a_row_removed_while_its_scroll_clip_shrinks_leaves_no_pixels() {
    removed_after_move(-400., false, true, Change::Remove);
}

#[test]
fn an_unchanged_jump_keeps_its_ink_damage_bounded() {
    removed_after_move(-400., false, false, Change::None);
}

#[test]
fn an_ordinary_scroll_still_copies() {
    removed_after_move(-16., false, false, Change::None);
}

#[test]
fn a_row_cleared_after_a_noncopying_jump_leaves_no_pixels() {
    removed_after_move(-400., false, false, Change::Clear);
}

#[test]
fn a_row_resized_after_a_noncopying_jump_erases_its_old_footprint() {
    removed_after_move(-400., false, false, Change::Narrow);
}

#[test]
fn a_row_hidden_after_a_noncopying_jump_erases_its_old_footprint() {
    removed_after_move(-400., false, false, Change::Hide);
}

#[derive(Clone, Copy, PartialEq)]
enum Change {
    None,
    Remove,
    Clear,
    Narrow,
    Hide,
}

fn removed_after_move(to: f32, isolated: bool, shrink_clip: bool, change: Change) {
    use zgx::scene::Transform;
    let (w, h) = (240., 180.);
    let mut ui = Ui::new(w, h);
    let offset = ui.signal(8.);
    ui.mount(
        scroll(offset).w(w).h(h).child(
            column()
                .w(w)
                .h(1000.)
                .child(div().absolute().top(64.).size(120., 14.).bg(rgb(0xffffff)))
                .child(div().absolute().top(500.).size(120., 14.).bg(rgb(0xff0000))),
        ),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let (content, old_row, viewport) = {
        let scene = ui.scene.borrow();
        let mut stack = vec![scene.root()];
        let content = loop {
            let id = stack.pop().expect("scroll content");
            if scene.is_scroll_content(id) {
                break id;
            }
            stack.extend_from_slice(scene.children(id));
        };
        let page = scene.children(content)[0];
        (
            content,
            scene.children(page)[0],
            scene.parent(content).unwrap(),
        )
    };
    if isolated {
        ui.scene.borrow_mut().set_isolated(viewport, true);
        ui.scene.borrow_mut().flush();
    }
    let full = [Rect::new(0., 0., w, h)];
    let mut partial = GpuRenderer::new(w as u32, h as u32).unwrap();
    partial.debug_allow_scroll_copies();
    partial.render(&ui.scene.borrow(), &full).unwrap();
    let before = partial.readback().unwrap();
    let report = {
        let mut scene = ui.scene.borrow_mut();
        // A virtual list changes its mounted range after applying its
        // translation. The old row's current box is outside the clip,
        // although its pixels still occupy the preceding frame.
        scene.set_transform(content, Transform { x: 0., y: to });
        match change {
            Change::None => {}
            Change::Remove => scene.remove(old_row),
            Change::Hide => {
                let mut style = scene.style(old_row);
                std::sync::Arc::make_mut(style.layout_options.get_or_insert_default()).visible =
                    Some(false);
                scene.set_style(old_row, style);
                scene.prepare_layout();
            }
            Change::Narrow => {
                let mut style = scene.style(old_row);
                style.width = Some(60.);
                scene.set_style(old_row, style);
                scene.prepare_layout();
            }
            Change::Clear => {
                let mut kind = scene.kind(old_row).clone();
                match &mut kind {
                    zgx::scene::NodeKind::Quad(q) | zgx::scene::NodeKind::Panel { quad: q, .. } => {
                        q.fill = zgx::scene::Color(0, 0, 0, 0)
                    }
                    _ => panic!("the row is a decorated box"),
                }
                scene.set_kind(old_row, kind);
            }
        }
        if shrink_clip {
            let mut style = scene.style(viewport);
            style.height = Some(40.);
            scene.set_style(viewport, style);
            scene.prepare_layout();
            assert_eq!(scene.bounds(viewport).height, 40.);
        }
        scene.flush()
    };
    let stats = partial.render(&ui.scene.borrow(), &report.damage).unwrap();
    if change == Change::None {
        let area: f32 = report.damage.iter().map(|r| r.width * r.height).sum();
        assert!(area < w * h, "unchanged content repainted the window");
        if (to + 8.).abs() < h {
            assert!(stats.scroll_copies > 0);
        }
    }
    let shown = partial.readback().unwrap();
    let mut whole = GpuRenderer::new(w as u32, h as u32).unwrap();
    whole.render(&ui.scene.borrow(), &full).unwrap();
    let expected = whole.readback().unwrap();
    let stale = shown
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        stale, 0,
        "removed row survived a noncopying move: {:?}",
        report.damage
    );
    // A layer may draw more than the requested region. Check the damage
    // contract itself so extra renderer work cannot hide an omission.
    let missed = before
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .filter(|(i, _)| {
            let pixel = Rect::new((i % w as usize) as f32, (i / w as usize) as f32, 1., 1.);
            !report.damage.iter().any(|r| r.intersects(pixel))
        })
        .count();
    if stats.scroll_copies == 0 {
        assert_eq!(
            missed, 0,
            "changed pixels fell outside damage {:?}",
            report.damage
        );
    }
}
