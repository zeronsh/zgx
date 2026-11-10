//! One changing numeric offset, applied to a fixed rail's thumb. Compare
//! BINDING=map|function at ROWS=10|400; TICKS=600 by default. Headless,
//! with setup and teardown outside cachegrind's measured interval.
#[path = "support/gate_marks.rs"]
mod gate_marks;
use zgx_script::{ScriptHost, Value, testing::TestApp};

fn main() {
    let binding = std::env::var("BINDING").unwrap_or_else(|_| "map".into());
    assert!(matches!(binding.as_str(), "map" | "function"));
    let rows: usize = std::env::var("ROWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);
    let ticks: usize = std::env::var("TICKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let app = gate_marks::outside(|| {
        let top = match binding.as_str() {
            "map" => "ui.map(offset,{from={0,100},to={4,104},clamp=true})",
            _ => "function() return 4 + math.clamp(offset:get(),0,100) end",
        };
        let source = format!(
            r#"
local offset = ui.state("offset",0)
local rows = {{}}
for i=1,{rows} do rows[i]=ui.div {{w=100,h=20,shrink=0}} end
return ui.column {{relative=true,w=300,h=220,
    ui.column {{shrink=0,table.unpack(rows)}},
    ui.div {{absolute=true,w=8,h=200,right=0,top=0,
        ui.div {{id="thumb",absolute=true,w=4,h=function() return 24 end,top={top}}},
    }},
}}
"#
        );
        let app = TestApp::new(|runtime| ScriptHost::inline(runtime, "app", &source));
        app.frames(2);
        for tick in 0..20 {
            app.host
                .set_state("offset", Value::Num(tick as f64))
                .unwrap();
            app.frame();
            app.ui.scene.borrow_mut().flush();
        }
        app
    });
    let calls = app.calls();
    let mut layout = 0;
    for tick in 0..ticks {
        app.host
            .set_state("offset", Value::Num((tick % 101) as f64))
            .unwrap();
        app.frame();
        layout += app.ui.scene.borrow_mut().flush().layout_nodes;
    }
    let lua = app.calls() - calls;
    if binding == "map" {
        assert_eq!(lua, 0, "source changes must not run Luau");
    }
    println!("{binding} rows={rows} ticks={ticks} lua={lua} layout_nodes={layout}");
    gate_marks::outside(|| drop(app));
}
