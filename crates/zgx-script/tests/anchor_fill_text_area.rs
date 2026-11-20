//! A Luau text area (`grow = 1, min_w = 0`, a chat composer's field) as a
//! popover's `anchor_fill` anchor shrinks in its row as it does without the
//! popover: its fixed control width (240) is no floor, and the button after
//! it stays in the row.
use zgx_script::testing::TestApp;

#[test]
fn an_anchor_fill_text_area_shrinks_as_in_its_row() {
    let mut app = TestApp::inline(
        r#"
        local function field(label, id)
          return ui.textarea {
            id = id, label = label, value = ui.state(label, "Also include Brazil"),
            grow = 1, min_w = 0, h = 28, wrap = true,
          }
        end
        return ui.column {
          w = 200,
          ui.row {
            w_full = true, gap = 6,
            ui.button { "a", w = 28, h = 28 },
            field("plain", "plain"),
            ui.button { "s", id = "s", w = 28, h = 28 },
          },
          ui.row {
            w_full = true, gap = 6,
            ui.button { "b", w = 28, h = 28 },
            ui.popover {
              open = function() return false end, anchor_fill = true,
              anchor = field("in popover", "pop"),
              ui.text { "menu" },
            },
            ui.button { "t", id = "t", w = 28, h = 28 },
          },
        }
        "#,
    );
    app.frames(2);
    let (plain, plain_last) = (app.bounds("plain"), app.bounds("s"));
    let (field, last) = (app.bounds("pop"), app.bounds("t"));
    assert!(plain_last.x + plain_last.width <= 200., "the plain row fits");
    assert_eq!(
        (field.width, last.x),
        (plain.width, plain_last.x),
        "as the popover's anchor: as wide, the last button in its row \
         (plain {plain:?}, popover {field:?})"
    );
}
