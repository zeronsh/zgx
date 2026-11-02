//! The break dump: the profiler's whole retention written out as text just
//! before the deliberate panic — every frame kept with its phases, damage,
//! reactive work, notes and slow effects, and the engine's own trace of the
//! writes and effect runs behind them.

use std::{
    fs::File,
    io::{self, BufWriter, Write},
};

use crate::record::{FrameRecord, History, Summary};

/// Write the dump: the summary, the markers, every frame kept, then the
/// engine's own traced frames (writes and effect runs by name, cause and
/// cost). Returns how many frames were written.
pub(super) fn write(
    path: &std::path::Path,
    history: &History,
    markers: &[(f64, String)],
    summary: &Summary,
) -> io::Result<usize> {
    let mut out = BufWriter::new(File::create(path)?);
    writeln!(out, "zgx-profiler crash dump")?;
    writeln!(
        out,
        "{} frames kept over {:.0}s, {} hud, {:.0} fps, p50 {:.1} p95 {:.1} p99 {:.1} ms, {} late, {:.0} MiB",
        summary.app_frames,
        summary.seconds,
        summary.hud_frames,
        summary.fps,
        summary.p50_ms,
        summary.p95_ms,
        summary.p99_ms,
        summary.dropped,
        summary.rss_mib,
    )?;
    for (at, name) in markers {
        writeln!(out, "marker {at:.1}ms  {name}")?;
    }
    if let Some(worst) = &summary.worst {
        let phases = worst
            .phases
            .iter()
            .map(|phase| format!("{} {:.0}", phase.name, phase.micros))
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(
            out,
            "worst {worst_ms:.1}ms at {at:.1}ms: {phases}",
            worst_ms = worst.interval_ms,
            at = worst.at_ms,
        )?;
    }
    writeln!(out, "frames:")?;
    for record in history.frames() {
        write_record(&mut out, record)?;
    }
    writeln!(out, "traced (zgx::trace: writes and effect runs):")?;
    for frame in zgx::trace::frames(zgx::trace::kept()) {
        let writes = frame
            .writes
            .iter()
            .map(|(signal, count)| format!("{signal:#x}x{count}"))
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(
            out,
            "  #{index} at {ended:.1}ms  writes: {writes}",
            index = frame.index,
            ended = frame.ended_ms,
            writes = if writes.is_empty() {
                "-".into()
            } else {
                writes
            },
        )?;
        for run in &frame.runs {
            writeln!(
                out,
                "    {:>10.0}us  {}{}",
                run.micros,
                run.name.as_deref().unwrap_or("(unnamed)"),
                match run.cause {
                    Some(cause) => format!("  from {cause:#x}"),
                    None => String::new(),
                },
            )?;
        }
        for note in &frame.notes {
            writeln!(out, "    why: {note}")?;
        }
    }
    out.flush()?;
    Ok(history.frames().len())
}

fn write_record(out: &mut BufWriter<File>, record: &FrameRecord) -> io::Result<()> {
    writeln!(out, "{}", crate::frame_log_line(record))?;
    for (signal, count) in &record.thrashing {
        writeln!(out, "    thrash {signal:#x} x{count}")?;
    }
    for run in &record.slow {
        writeln!(
            out,
            "    slow  {} {run_us:.0}us{cause}",
            run.name,
            run_us = run.micros,
            cause = match run.cause {
                Some(cause) => format!("  from {cause:#x}"),
                None => String::new(),
            },
        )?;
    }
    for note in &record.notes {
        writeln!(out, "    why   {note}")?;
    }
    for rect in record.damage.iter().take(12) {
        writeln!(
            out,
            "    rect  {:.1} {:.1} {:.1} {:.1}",
            rect.x, rect.y, rect.width, rect.height
        )?;
    }
    Ok(())
}
