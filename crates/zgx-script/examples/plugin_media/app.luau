-- An app that leaves a slot to its plugin, a script of its own (zgx.json's
-- `scripts`), which plays sound and draws a 3D scene. Run it with
-- `cargo run -p zgx-run --features audio -- crates/zgx-script/examples/plugin_media`.
return ui.column {
	gap = 12,
	p = 16,
	ui.text { "The app's own text; the rest is the plugin's." },
	ui.slot("media", { id = "media" }),
}
