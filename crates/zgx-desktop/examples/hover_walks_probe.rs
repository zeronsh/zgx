//! How often a window looks for what is under a resting pointer while the
//! content under it moves (`scripts/hover_walks_smoke.py`): a chat's
//! transcript following its end as a reply streams in, the real pointer
//! resting over it. Each frame that moves geometry looks for hover once
//! (`Ui::refresh_hover_moved`), and the cursor after drawing
//! (`sync_cursor`) answers from that walk: a frame walking the scene twice
//! is a miss of the hit memo between them. With `HOVER_WALKS_DRIVEN=1`, a
//! driver then moves the pointer into the interface alone (as `zgx inspect`
//! and agents do), elsewhere than the real one rests: the cursor follows
//! it, as hover does, from the same walk. Prints `AT x y` (where to rest
//! the pointer, in the window's points), then `WALKS` with the frames
//! counted, those that walked, and those that walked more than once.
use std::time::{Duration, Instant};
use zgx_desktop::{Application, WindowOptions};
use zgx_script::ScriptHost;

const APP: &str = r##"
local reply = ui.state("reply", "")
local words = { "The ", "invoice ", "total ", "is ", "summed ", "from ", "its ", "lines, ", "then ", "rounded. " }
local n = 0
-- A model's tokens, a few each 50 ms; a long reply starts over.
ui.every(50, function()
	n += 1
	if #reply:peek() > 3000 then
		reply:set("")
	end
	reply:append(words[n % #words + 1] .. words[(n + 3) % #words + 1])
end)
local rows = {}
for i = 1, 40 do
	table.insert(rows, { id = `m{i}` })
end
table.insert(rows, { id = "reply" })
local function message(row: any)
	local body = if row.id == "reply"
		then ui.text { function()
			return reply:get()
		end, wrap = true }
		else ui.text { `An earlier message, {row.id}, of a line or two of text that wraps.`, wrap = true }
	return ui.column {
		gap = 4,
		w_full = true,
		ui.text { if row.id == "reply" then "Assistant" else "Someone" },
		body,
		ui.button { "Copy", on_click = function() end, cursor = "pointer" },
	}
end
return ui.list {
	id = "transcript",
	data = ui.state("rows", rows),
	key = function(row: any): string
		return row.id
	end,
	row = message,
	offset = ui.state("offset", math.huge),
	measured = true,
	follow = true,
	height = 64,
	w_full = true,
	h_full = true,
}
"##;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: "zgx hover walks".into(),
            width: 600.,
            height: 500.,
            ..Default::default()
        })
        .run(|cx| {
            let host = ScriptHost::inline(&cx.ui.runtime, "app", APP);
            if let Err(error) = host.reload() {
                eprintln!("{error}");
                std::process::exit(1);
            }
            cx.render(host.view());
            let ui = cx.ui.downgrade();
            let frames = cx.frames.clone();
            let window = cx.window.clone();
            cx.tasks.spawn(async move {
                let _host = host;
                zgx::timer::sleep(Duration::from_millis(500)).await;
                println!("AT 300 250");
                println!("READY");
                // The pointer's arrival, then a second for hover to settle.
                loop {
                    let Some(ui) = ui.upgrade() else { return };
                    if ui.input.pointer().is_some() {
                        break;
                    }
                    drop(ui);
                    zgx::timer::sleep(Duration::from_millis(50)).await;
                }
                zgx::timer::sleep(Duration::from_secs(1)).await;
                // A driver's move into the interface alone, as zgx inspect's.
                if std::env::var_os("HOVER_WALKS_DRIVEN").is_some()
                    && let Some(mut ui) = ui.upgrade()
                {
                    ui.dispatch(zgx::input::InputEvent::PointerMove { x: 300., y: 350. });
                }
                zgx::timer::sleep(Duration::from_millis(300)).await;
                let walks = || ui.upgrade().map_or(0, |ui| ui.scene.borrow().hit_walks());
                let (mut counted, mut walked, mut twice, mut most) = (0, 0, 0, 0);
                let mut last = walks();
                let until = Instant::now() + Duration::from_secs(3);
                while Instant::now() < until {
                    frames.next().await;
                    let now = walks();
                    let these = now - last;
                    last = now;
                    counted += 1;
                    walked += u64::from(these > 0);
                    twice += u64::from(these > 1);
                    most = most.max(these);
                }
                println!("WALKS frames={counted} walked={walked} twice={twice} most={most}");
                window.close();
            });
        })
}
