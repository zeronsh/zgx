//! `ui.decode_image`: a picture's bytes in base64 (a server's, the host's)
//! become a `Bitmap` that `ui.image` shows; text that isn't base64 or bytes
//! that aren't a picture reject its promise.
mod common;
use base64::Engine as _;
use common::Harness;

/// A 2×2 PNG.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 6, 0,
    0, 0, 114, 182, 13, 36, 0, 0, 0, 17, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240, 31, 132,
    25, 96, 12, 0, 71, 202, 7, 249, 103, 89, 110, 183, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
    130,
];

#[test]
fn a_pictures_bytes_become_a_bitmap_an_image_shows() {
    let encoded = base64::engine::general_purpose::STANDARD.encode(PNG);
    let code = format!(
        r#"
local picture = ui.state("picture", nil)
local said = ui.state("said", "")
local _ = ui.task(function()
    local bitmap = ui.decode_image({encoded:?}):await()
    said:set(`{{bitmap.w}}x{{bitmap.h}}`)
    picture:set(bitmap)
end)
return ui.column {{
    ui.text {{ id = "said", function() return said:get() end }},
    ui.image {{ id = "pic", src = picture, w = 20, h = 20 }},
}}"#
    );
    let mut h = Harness::inline(&code);
    h.wait_until(|h| !h.text("said").is_empty() || h.host.error().get().is_some());
    assert_eq!(h.host.error().get(), None);
    assert_eq!(h.text("said"), "2x2");
    assert_eq!(h.bounds("pic").width, 20.);
}

#[test]
fn text_that_isnt_base64_or_bytes_that_arent_a_picture_reject_it() {
    for (base64, said) in [("not base64!", "isn't base64"), ("aGVsbG8=", "a picture:")] {
        let code = format!(
            "local _ = ui.task(function() ui.decode_image({base64:?}):await() end)\nreturn ui.text {{ \"x\" }}"
        );
        let mut h = Harness::inline(&code);
        h.wait_until(|h| h.host.error().get().is_some());
        let error = h.host.error().get().unwrap_or_default();
        assert!(error.contains(said), "{error}");
    }
}
