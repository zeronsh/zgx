//! `ui.markdown` with `lazy`: a long reply landing whole comes back at once
//! as one entry a block, those not parsed yet standing in (`t = "pending"`
//! with `hint`, `lines`, `from`, `to`), and is parsed over frames, the end
//! first, the computed reading it running again as blocks come.
mod common;
use common::Harness;

const APP: &str = r#"
local reply = ui.state("reply", "")
local runs = ui.state("runs", 0)
local blocks = ui.computed(function()
    runs:set(runs:peek() + 1)
    return ui.markdown(reply:get(), { key = "reply", lazy = true })
end)
local function counts()
    local list = blocks:get()
    local pending = 0
    for _, b in list do
        if b.t == "pending" then pending += 1 end
    end
    local last = list[#list]
    return `{#list} {pending} {if last then last.t else "none"}`
end
return ui.column {
    ui.button { id = "land", w = 20, h = 20, on_click = function()
        local parts = {}
        for n = 1, 200 do
            table.insert(parts, `## Part {n}\n\nA paragraph {n} with **bold** and \`code\`.\n`)
        end
        table.insert(parts, "The end.")
        reply:set(table.concat(parts, "\n"))
    end },
    ui.text { id = "counts", counts },
    ui.text { id = "runs", "{runs}" },
}
"#;

#[test]
fn a_long_reply_landing_whole_parses_over_frames_the_end_first() {
    let mut h = Harness::inline(APP);
    h.frames(1);
    h.click("land");
    h.ui.prepare_frame();
    // 401 blocks at once (200 headings, 200 paragraphs, the end), most
    // standing in; the end itself parsed.
    let counts = h.text("counts");
    let parts: Vec<&str> = counts.split(' ').collect();
    assert_eq!(parts[0], "401", "{counts}");
    assert!(parts[1].parse::<usize>().unwrap() > 300, "{counts}");
    assert_eq!(parts[2], "p", "{counts}");
    let runs: usize = h.text("runs").parse().unwrap();
    // Parsed over ticks, each telling the computed.
    assert!(
        h.wait_until(|h| h.text("counts") == "401 0 p"),
        "{}",
        h.text("counts")
    );
    let after: usize = h.text("runs").parse().unwrap();
    assert!(after > runs + 1, "ran {runs} then {after}");
    assert_eq!(h.error(), None);
}
