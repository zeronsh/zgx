//! `ui.load_font`: a font file a script loads is shaped with by name, where
//! text in its family fell back before; a file that is not a font is an
//! error; a plugin loads one only if its host grants `Capability::Fonts`.
mod common;
use common::Harness;
use std::{cell::RefCell, rc::Rc, sync::Arc};
use zgx::platform::{FilePicker, Pending, Platform, PlatformService};
use zgx_script::ScriptHost;

const FONT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../zgx-desktop/examples/support/fonts/InstrumentSerif-Regular.ttf"
);

/// A window's text: fonts added as a desktop window adds them, and every
/// text measured again.
struct Fonts {
    services: zgx_gpu::TextServices,
    scene: RefCell<Option<Rc<RefCell<zgx::scene::Scene>>>>,
}
impl Platform for Fonts {
    fn clipboard_text(&self) -> Option<String> {
        None
    }
    fn set_clipboard_text(&self, _: String) -> bool {
        false
    }
    fn open_files(&self, _: FilePicker) -> Pending<Result<Vec<std::path::PathBuf>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn save_file(&self, _: FilePicker) -> Pending<Result<Option<std::path::PathBuf>, String>> {
        Box::pin(async { Ok(None) })
    }
    fn add_font(&self, bytes: Arc<[u8]>) -> Result<Vec<String>, String> {
        let font = zgx_gpu::text::FontData::new(bytes)?;
        font.install(&mut self.services.text_system().borrow_mut());
        self.services.release_unused();
        if let Some(scene) = &*self.scene.borrow() {
            scene.borrow_mut().fonts_replaced();
        }
        Ok(font.families().to_vec())
    }
}

fn app(code: String, plugin: Option<(String, bool)>) -> Harness {
    let fonts = Rc::new(Fonts {
        services: zgx_gpu::TextServices::new(),
        scene: RefCell::new(None),
    });
    let platform = fonts.clone();
    let h = Harness::wrapped(
        move |runtime| {
            let host = ScriptHost::inline(runtime, "app", code);
            match plugin {
                Some((code, true)) => host
                    .plugin("fonts.luau", code)
                    .grant("fonts.luau", zgx_script::Capability::Fonts),
                Some((code, false)) => host.plugin("fonts.luau", code),
                None => host,
            }
        },
        move |view| zgx::compose::provide(PlatformService(platform), view),
    );
    fonts.services.install(&mut h.ui.scene.borrow_mut());
    *fonts.scene.borrow_mut() = Some(h.ui.scene.clone());
    h
}
fn settled(h: &mut Harness) {
    for _ in 0..20 {
        h.frames(1);
        h.settle();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

const MEASURES: &str = r##"
local said = ui.state("said", "")
local loaded = ui.state("loaded", false)
local seen = 0
return ui.column { w = 400, h = 300,
    ui.text { id = "label", function() return said:get() end },
    ui.button { id = "load", "load", on_click = function()
        ui.task(function()
            local families = ui.load_font(PATH):await()
            said:set(table.concat(families, ","))
            loaded:set(true)
        end)
    end },
    ui.button { id = "read", "read", on_click = function() said:set(tostring(seen)) end },
    ui.row { ui.text { id = "sample", font = "Instrument Serif", text_size = 20, "Handgloves" } },
    ui.canvas { id = "c", w = 300, h = 40, draw = function(c)
        local _ = loaded:get()
        seen = c:measure_text("Handgloves", { font = "Instrument Serif", size = 20 })
    end },
}"##;

#[test]
fn a_loaded_font_measures_and_lays_out_text_in_its_family() {
    let mut h = app(MEASURES.replace("PATH", &format!("{FONT:?}")), None);
    settled(&mut h);
    h.click("read");
    settled(&mut h);
    let read = h.text("label");
    let fallback: f32 = read.parse().unwrap();
    let laid_out = h.bounds("sample").width;
    h.click("load");
    h.wait_until(|h| h.text("label") != read || h.host.error().get().is_some());
    assert_eq!(
        h.text("label"),
        "Instrument Serif",
        "{:?}",
        h.host.error().get()
    );
    h.click("read");
    settled(&mut h);
    let loaded: f32 = h.text("label").parse().unwrap();
    assert!(
        fallback > 0. && (loaded - fallback).abs() > 1.,
        "measured {fallback} before, {loaded} after"
    );
    let relaid = h.bounds("sample").width;
    assert!(
        (relaid - loaded).abs() < 1. && (relaid - laid_out).abs() > 1.,
        "laid out {laid_out} before, {relaid} after ({loaded} measured)"
    );
    assert_eq!(h.host.error().get(), None);
}

/// A font's bytes in base64 (one a server or the host's call handed over)
/// load as its file does.
#[test]
fn a_font_from_its_bytes_measures_and_lays_out_text_in_its_family() {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(std::fs::read(FONT).unwrap());
    let code = MEASURES.replace("PATH", &format!("{{ base64 = {encoded:?} }}"));
    let mut h = app(code, None);
    settled(&mut h);
    h.click("read");
    settled(&mut h);
    let fallback: f32 = h.text("label").parse().unwrap();
    h.click("load");
    h.wait_until(|h| h.text("label") == "Instrument Serif" || h.host.error().get().is_some());
    assert_eq!(h.host.error().get(), None);
    h.click("read");
    settled(&mut h);
    let loaded: f32 = h.text("label").parse().unwrap();
    assert!(
        (loaded - fallback).abs() > 1.,
        "measured {fallback} before, {loaded} after"
    );
    assert!((h.bounds("sample").width - loaded).abs() < 1.);
}

#[test]
fn bytes_that_are_not_base64_or_not_a_font_are_an_error() {
    for (base64, said) in [
        ("not base64!", "isn't base64"),
        ("aGVsbG8=", "invalid or unsupported"),
    ] {
        let code = format!(
            "local _ = ui.task(function() ui.load_font({{ base64 = {base64:?} }}):await() end)
return ui.text {{ \"x\" }}"
        );
        let mut h = app(code, None);
        h.wait_until(|h| h.host.error().get().is_some());
        let error = h.host.error().get().unwrap_or_default();
        assert!(error.contains(said), "{error}");
    }
}

#[test]
fn a_file_that_is_not_a_font_is_an_error() {
    let dir = common::temp_dir("load-font");
    let file = dir.join("not-a-font.ttf");
    std::fs::write(&file, "plain text, not a font").unwrap();
    let code = format!(
        "local _ = ui.task(function() ui.load_font({:?}):await() end)\nreturn ui.text {{ \"x\" }}",
        file.to_string_lossy()
    );
    let mut h = app(code, None);
    h.wait_until(|h| h.host.error().get().is_some());
    let error = h.host.error().get().unwrap_or_default();
    assert!(
        error.contains("not-a-font.ttf") && error.contains("invalid or unsupported"),
        "{error}"
    );
}

#[test]
fn a_plugin_loads_a_font_only_if_granted_and_only_its_own() {
    let plugin = format!("local _ = ui.task(function() ui.load_font({FONT:?}):await() end)");
    let shown = "return ui.text { \"app\" }".to_owned();
    let mut h = app(shown.clone(), Some((plugin.clone(), false)));
    settled(&mut h);
    let error = h.host.error().get().unwrap_or_default();
    assert!(error.contains("Capability::Fonts"), "{error}");
    // Granted, it still reads only inside its own directory.
    let mut h = app(shown, Some((plugin, true)));
    settled(&mut h);
    let error = h.host.error().get().unwrap_or_default();
    assert!(
        error.contains("a plugin's fonts come from its directory"),
        "{error}"
    );
}
