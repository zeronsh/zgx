//! Where the pointer passed between two moves a host folds into one (a
//! platform's moves of one turn, `InputDispatcher::pointer_passed`): hover
//! goes there, so a cell crossed in between hears its enter and leave, and
//! no move.
use std::{cell::RefCell, rc::Rc};
use zgx::{
    input::{InputDispatcher, InputEvent, NodeInput},
    scene::{Color, Layout, NodeId, NodeKind, Scene, Style},
};

/// A row of three cells, each 50 points wide.
fn cells() -> (Rc<RefCell<Scene>>, [NodeId; 3]) {
    let mut s = Scene::new(150., 50.);
    let root = s.root();
    s.set_kind(root, NodeKind::Container(Layout::Row));
    let style = Style {
        width: Some(50.),
        height: Some(50.),
        flex_shrink: 0.,
        ..Default::default()
    };
    let cells =
        [0, 1, 2].map(|_| s.append(root, NodeKind::Rect(Color(1, 2, 3, 255)), style.clone()));
    s.flush();
    (Rc::new(RefCell::new(s)), cells)
}

/// What each cell heard, as `cell event`.
fn listen(
    input: &InputDispatcher,
    cells: [NodeId; 3],
) -> (Rc<RefCell<Vec<String>>>, Vec<impl Sized>) {
    let heard = Rc::new(RefCell::new(Vec::new()));
    let handles = cells
        .iter()
        .enumerate()
        .map(|(i, &cell)| {
            let heard = heard.clone();
            input.register(cell, NodeInput::default(), move |e| {
                let what = match e.event {
                    InputEvent::PointerEnter => "enter",
                    InputEvent::PointerLeave => "leave",
                    InputEvent::PointerMove { .. } => "move",
                    _ => return,
                };
                heard.borrow_mut().push(format!("{i} {what}"));
            })
        })
        .collect();
    (heard, handles)
}

#[test]
fn a_cell_crossed_between_two_moves_of_a_turn_hears_its_enter() {
    let (scene, cells) = cells();
    let input = InputDispatcher::new();
    let (heard, _handles) = listen(&input, cells);
    input.dispatch(&scene, InputEvent::PointerMove { x: 25., y: 25. });
    heard.borrow_mut().clear();
    // One turn: a move over the middle cell, then one past it, folded into
    // the last; the first passed on the way.
    input.pointer_passed(&scene, 75., 25.);
    input.dispatch(&scene, InputEvent::PointerMove { x: 125., y: 25. });
    assert_eq!(
        *heard.borrow(),
        ["0 leave", "1 enter", "1 leave", "2 enter", "2 move"],
        "the middle cell is entered and left, and hears no move"
    );
    assert_eq!(input.hovered(), Some(cells[2]));
}

#[test]
fn passing_where_the_move_then_lands_tells_nothing_twice() {
    let (scene, cells) = cells();
    let input = InputDispatcher::new();
    let (heard, _handles) = listen(&input, cells);
    input.dispatch(&scene, InputEvent::PointerMove { x: 25., y: 25. });
    heard.borrow_mut().clear();
    input.pointer_passed(&scene, 30., 25.);
    input.pointer_passed(&scene, 80., 25.);
    input.pointer_passed(&scene, 90., 25.);
    input.dispatch(&scene, InputEvent::PointerMove { x: 95., y: 25. });
    assert_eq!(*heard.borrow(), ["0 leave", "1 enter", "1 move"]);
}
