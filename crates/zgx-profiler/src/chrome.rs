use std::{
    collections::VecDeque,
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
};

use crate::record::{FrameRecord, Segment, WorstFrame};

/// Writes `frames` as a Chrome trace (`chrome://tracing`, Perfetto), one
/// process with three tracks: each frame's phases as complete events on the
/// UI thread, its damage and reactive work as instant events and counters,
/// the costliest named effects that ran in it and why it happened as
/// instants, the marker-delimited `segments` of the capture as slices of
/// their own thread, and the `worst` frame kept as one instant with its
/// split. Returns how many frames were written.
pub fn write(
    path: impl AsRef<Path>,
    frames: &VecDeque<FrameRecord>,
    markers: &[(f64, String)],
    segments: &[Segment],
    worst: Option<&WorstFrame>,
) -> io::Result<usize> {
    let mut out = BufWriter::new(File::create(path)?);
    out.write_all(b"{\"displayTimeUnit\":\"ms\",\"traceEvents\":[\n")?;
    let mut first = true;
    let mut event = |out: &mut BufWriter<File>, body: String| -> io::Result<()> {
        if !first {
            out.write_all(b",\n")?;
        }
        first = false;
        out.write_all(body.as_bytes())
    };
    event(
        &mut out,
        "{\"name\":\"process_name\",\"ph\":\"M\",\"pid\":1,\"args\":{\"name\":\"zgx app (zgx-profiler)\"}}"
            .into(),
    )?;
    event(
        &mut out,
        "{\"name\":\"thread_name\",\"ph\":\"M\",\"pid\":1,\"tid\":1,\"args\":{\"name\":\"UI thread\"}}"
            .into(),
    )?;
    event(
        &mut out,
        "{\"name\":\"thread_name\",\"ph\":\"M\",\"pid\":1,\"tid\":2,\"args\":{\"name\":\"segments\"}}"
            .into(),
    )?;
    for segment in segments {
        let phases: Vec<String> = segment
            .phase_micros
            .iter()
            .take(6)
            .map(|phase| format!("\"{}\":{:.0}", escape(phase.name), phase.micros))
            .collect();
        event(
            &mut out,
            format!(
                "{{\"name\":\"{}\",\"cat\":\"segment\",\"ph\":\"X\",\"pid\":1,\"tid\":2,\"ts\":{:.1},\"dur\":{:.1},\"args\":{{\"frames\":{},\"fps\":{:.1},\"p50_ms\":{:.1},\"p95_ms\":{:.1},\"damage_px2\":{:.0},\"phases\":{{{}}}}}}}",
                escape(if segment.name.is_empty() {
                    "unmarked"
                } else {
                    &segment.name
                }),
                segment.from_ms * 1e3,
                (segment.to_ms - segment.from_ms) * 1e3,
                segment.frames,
                segment.fps,
                segment.p50_ms,
                segment.p95_ms,
                segment.damage_area,
                phases.join(","),
            ),
        )?;
    }
    for frame in frames {
        let total = frame.total_micros();
        let start = frame.at_ms * 1e3 - f64::from(total);
        let mut at = start;
        for phase in &frame.phases {
            let micros = f64::from(phase.micros);
            event(
                &mut out,
                format!(
                    "{{\"name\":\"{}\",\"cat\":\"frame\",\"ph\":\"X\",\"pid\":1,\"tid\":1,\"ts\":{at:.1},\"dur\":{micros:.1},\"args\":{{\"instructions\":{}}}}}",
                    escape(phase.name),
                    phase.instructions,
                ),
            )?;
            at += micros;
        }
        event(
            &mut out,
            format!(
                "{{\"name\":\"frame\",\"cat\":\"frame\",\"ph\":\"i\",\"s\":\"t\",\"pid\":1,\"tid\":1,\"ts\":{:.1},\"args\":{{\"rects\":{},\"damage_px2\":{:.1},\"layout\":{},\"paint\":{},\"composite\":{},\"full\":{},\"instructions\":{},\"writes\":{},\"effects\":{},\"cascade\":{:.2},\"gpu_us\":{},\"hud_only\":{},\"damage\":[{}]}}}}",
                frame.at_ms * 1e3,
                frame.damage.len(),
                frame.damage_area,
                frame.layout_nodes,
                frame.paint_nodes,
                frame.composite_nodes,
                frame.full,
                frame.instructions,
                frame.writes,
                frame.effects,
                frame.cascade,
                frame
                    .gpu_micros
                    .map_or_else(|| "null".to_string(), |micros| format!("{micros:.1}")),
                frame.hud_only,
                frame
                    .damage
                    .iter()
                    .take(8)
                    .map(|rect| format!(
                        "[{:.1},{:.1},{:.1},{:.1}]",
                        rect.x, rect.y, rect.width, rect.height
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        )?;
        for (name, value) in [
            ("frame_ms", f64::from(frame.interval_ms)),
            ("damage_px2", f64::from(frame.damage_area)),
            ("signal_writes", f64::from(frame.writes)),
            ("effects", f64::from(frame.effects)),
            ("instructions", frame.instructions as f64),
        ] {
            event(
                &mut out,
                format!(
                    "{{\"name\":\"{name}\",\"ph\":\"C\",\"pid\":1,\"tid\":1,\"ts\":{:.1},\"args\":{{\"{name}\":{value:.1}}}}}",
                    frame.at_ms * 1e3
                ),
            )?;
        }
        for run in &frame.slow {
            event(
                &mut out,
                format!(
                    "{{\"name\":\"effect: {}\",\"cat\":\"effect\",\"ph\":\"i\",\"s\":\"t\",\"pid\":1,\"tid\":1,\"ts\":{:.1},\"args\":{{\"us\":{:.1},\"signal\":{}}}}}",
                    escape(&run.name),
                    frame.at_ms * 1e3,
                    run.micros,
                    run.cause
                        .map_or_else(|| "null".to_string(), |signal| format!("\"{signal:#x}\""))
                ),
            )?;
        }
        for (signal, count) in &frame.thrashing {
            event(
                &mut out,
                format!(
                    "{{\"name\":\"signal thrashing\",\"cat\":\"reactive\",\"ph\":\"i\",\"s\":\"t\",\"pid\":1,\"tid\":1,\"ts\":{:.1},\"args\":{{\"signal\":\"{signal:#x}\",\"writes\":{count}}}}}",
                    frame.at_ms * 1e3
                ),
            )?;
        }
        for note in &frame.notes {
            event(
                &mut out,
                format!(
                    "{{\"name\":\"{}\",\"cat\":\"why\",\"ph\":\"i\",\"s\":\"t\",\"pid\":1,\"tid\":1,\"ts\":{:.1}}}",
                    escape(note),
                    frame.at_ms * 1e3
                ),
            )?;
        }
    }
    for (at_ms, name) in markers {
        event(
            &mut out,
            format!(
                "{{\"name\":\"{}\",\"cat\":\"marker\",\"ph\":\"i\",\"s\":\"t\",\"pid\":1,\"tid\":1,\"ts\":{:.1}}}",
                escape(name),
                at_ms * 1e3
            ),
        )?;
    }
    if let Some(worst) = worst {
        let phases: Vec<String> = worst
            .phases
            .iter()
            .map(|phase| format!("\"{}\":{:.0}", escape(phase.name), phase.micros))
            .collect();
        event(
            &mut out,
            format!(
                "{{\"name\":\"worst frame\",\"cat\":\"frame\",\"ph\":\"i\",\"s\":\"g\",\"pid\":1,\"tid\":1,\"ts\":{:.1},\"args\":{{\"interval_ms\":{:.1},\"instructions\":{},\"phases\":{{{}}},\"why\":[{}]}}}}",
                worst.at_ms * 1e3,
                worst.interval_ms,
                worst.instructions,
                phases.join(","),
                worst
                    .notes
                    .iter()
                    .map(|note| format!("\"{}\"", escape(note)))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        )?;
    }
    out.write_all(b"\n]}\n")?;
    out.flush()?;
    Ok(frames.len())
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::PhaseCost;

    #[test]
    fn the_trace_is_valid_json_with_every_kind_of_event() {
        let frame = FrameRecord {
            at_ms: 10.,
            interval_ms: 16.7,
            phases: vec![PhaseCost {
                name: "paint",
                micros: 400.,
                instructions: 12_000,
            }],
            instructions: 12_000,
            slow: vec![crate::record::SlowRun {
                name: "app.luau:30 \"effect\"".into(),
                micros: 900.,
                cause: Some(0x10),
            }],
            notes: vec!["a scroll repainted".into()],
            composite_nodes: 4,
            full: true,
            ..Default::default()
        };
        let mut frames = VecDeque::new();
        frames.push_back(frame);
        let segments = vec![Segment {
            name: "scenario: on".into(),
            from_ms: 0.,
            to_ms: 10.,
            frames: 1,
            fps: 60.,
            ..Default::default()
        }];
        let worst = WorstFrame {
            at_ms: 10.,
            interval_ms: 40.,
            ..Default::default()
        };
        let path = std::env::temp_dir().join("zgx-profiler-chrome-test.json");
        let written = write(
            &path,
            &frames,
            &[(5., "scenario: on".into())],
            &segments,
            Some(&worst),
        )
        .expect("the trace is written");
        assert_eq!(written, 1);
        let text = std::fs::read_to_string(&path).expect("the trace is read back");
        assert!(text.starts_with("{\"displayTimeUnit\":\"ms\",\"traceEvents\":["));
        assert!(text.ends_with("]}\n"));
        for needed in [
            "\"effect: app.luau:30 \\\"effect\\\"\"",
            "\"cat\":\"why\"",
            "\"cat\":\"segment\"",
            "\"worst frame\"",
            "\"instructions\":12000",
            "\"full\":true",
            "\"signal\":\"0x10\"",
        ] {
            assert!(text.contains(needed), "missing {needed} in the trace");
        }
        let value: serde_json::Value = serde_json::from_str(&text).expect("the trace parses");
        assert!(value["traceEvents"].as_array().unwrap().len() > 10);
        std::fs::remove_file(&path).ok();
    }
}
