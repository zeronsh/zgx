use std::{
    collections::VecDeque,
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
};

use crate::record::FrameRecord;

/// Writes `frames` as a Chrome trace (`chrome://tracing`, Perfetto), one thread
/// on one process: each frame's phases as complete events, its damage and
/// reactive work as instant events and counters, and the markers taken while
/// recording. Returns how many frames were written.
pub fn write(
    path: impl AsRef<Path>,
    frames: &VecDeque<FrameRecord>,
    markers: &[(f64, String)],
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
    for frame in frames {
        let total = frame.total_micros();
        let start = frame.at_ms * 1e3 - f64::from(total);
        let mut at = start;
        for (phase, micros) in &frame.phases {
            let micros = f64::from(*micros);
            event(
                &mut out,
                format!(
                    "{{\"name\":\"{}\",\"cat\":\"frame\",\"ph\":\"X\",\"pid\":1,\"tid\":1,\"ts\":{at:.1},\"dur\":{micros:.1}}}",
                    escape(phase)
                ),
            )?;
            at += micros;
        }
        event(
            &mut out,
            format!(
                "{{\"name\":\"frame\",\"cat\":\"frame\",\"ph\":\"i\",\"s\":\"g\",\"pid\":1,\"tid\":1,\"ts\":{:.1},\"args\":{{\"rects\":{},\"damage_px2\":{:.1},\"layout\":{},\"paint\":{},\"writes\":{},\"effects\":{},\"cascade\":{:.2},\"gpu_us\":{},\"hud_only\":{},\"damage\":[{}]}}}}",
                frame.at_ms * 1e3,
                frame.damage.len(),
                frame.damage_area,
                frame.layout_nodes,
                frame.paint_nodes,
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
        ] {
            event(
                &mut out,
                format!(
                    "{{\"name\":\"{name}\",\"ph\":\"C\",\"pid\":1,\"tid\":1,\"ts\":{:.1},\"args\":{{\"{name}\":{value:.1}}}}}",
                    frame.at_ms * 1e3
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
