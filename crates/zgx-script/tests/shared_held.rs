//! A shared value read fresh on every read (`ui.shared(key, nil):get()` in
//! a reader, with no handle kept), before the host's first publish: the
//! host's publish still reaches the reader, after the scripts' garbage is
//! collected.
mod common;

use common::Harness;
use zgx_script::Value;

const READER: &str = "return ui.column {\n\
   ui.text { id = \"open\", function()\n\
     local v = ui.shared(\"note_open\", nil):get()\n\
     return v == nil and \"none\" or tostring(v)\n\
   end },\n\
 }\n";

#[test]
fn a_reader_with_no_handle_hears_the_hosts_first_publish() {
    let mut h = Harness::inline(READER);
    assert_eq!(h.error(), None);
    h.frames(2);
    assert_eq!(h.text("#open"), "none");
    // Nothing holds the handle the reader made: collected.
    zgx::idle::trim(&h.ui.runtime);
    h.host.collect_garbage();
    h.frames(2);
    h.host
        .shared("note_open", Value::Nil)
        .set(Value::from("doc-1"));
    h.frames(2);
    assert_eq!(
        h.text("#open"),
        "doc-1",
        "the host's publish reached the reader"
    );
}

#[test]
fn an_effect_with_no_handle_hears_the_hosts_first_publish() {
    let mut h = Harness::inline(
        "ui.effect(function()\n\
           local v = ui.shared(\"note_open\", nil):get()\n\
           if v ~= nil then ui.state(\"heard\", \"\"):set(v) end\n\
         end)\n\
         local heard = ui.state(\"heard\", \"\")\n\
         return ui.text { id = \"heard\", function() return heard:get() end }\n",
    );
    assert_eq!(h.error(), None);
    h.frames(2);
    zgx::idle::trim(&h.ui.runtime);
    h.host.collect_garbage();
    h.frames(2);
    h.host
        .shared("note_open", Value::Nil)
        .set(Value::from("doc-2"));
    h.frames(2);
    assert_eq!(h.text("#heard"), "doc-2", "the effect ran again");
}

#[test]
fn a_reader_with_no_handle_hears_the_host_after_a_hot_reload() {
    let mut h = Harness::inline(READER);
    assert_eq!(h.error(), None);
    h.frames(2);
    h.host.reload().unwrap();
    h.frames(2);
    assert_eq!(h.text("#open"), "none");
    zgx::idle::trim(&h.ui.runtime);
    h.host.collect_garbage();
    h.frames(2);
    h.host
        .shared("note_open", Value::Nil)
        .set(Value::from("doc-3"));
    h.frames(2);
    assert_eq!(
        h.text("#open"),
        "doc-3",
        "the reloaded reader heard the publish"
    );
}
