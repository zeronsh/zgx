//! `zgx-video`, also run as `zgx video`: render a composition, list a
//! script's, take a still.
use serde_json::{Value as Json, json};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use zgx_video::{Format, RenderOptions};

const USAGE: &str = "usage: zgx video render <dir> [composition] <out> [options]
       zgx video still <dir> [composition] <frame> <out.png> [--scale S] [--props JSON] [--thumbnail WIDTH]
       zgx video list <dir> [--props JSON]
       zgx video studio <dir> [--preview-scale S]

<dir> holds a script returning video.compositions { … } (video.luau or app.luau).
<out>'s extension says the format: .mp4 is H.264, .mov ProRes 4444, .webm VP9
(alpha kept), .gif a GIF,
and a directory or a path holding %04d is a PNG for each frame. H.264, HEVC,
ProRes, VP9 and AV1 need ffmpeg on the PATH.

  --codec h264|hevc|prores|vp9|av1|png|gif   the format, over the extension
  --fps N             frames per second of the file (the composition plays at
                      its own speed, with more or fewer frames)
  --scale S           pixels per point (1)
  --frames A-B        only these frames of the file, inclusive
  --props JSON        props over its defaults; @file.json reads a file
  --dataset FILE      a JSON list: one video for each row's props, <out> holding
                      {id} or {index}, or a field's name as {title}
  --transparent       keep the background clear (ProRes, PNG, GIF)
  --quality N         constant quality, lower better (16)
  --motion-blur N     blend N samples into each frame
  --shutter F         how much of a frame's time they span (0.5)
  --mute              leave its sounds out
  --concurrency N     render N chunks of frames at once, each on its own thread
                      with its own stage and encoder (the CPUs' count, at
                      most 4, by default; 1 renders one frame after another)
  --every-nth-frame N keep one frame in N, the file's rate divided by N
  --thumbnail W       (still) no wider than W pixels
  --quiet             no progress";

struct Parsed {
    thumbnail: Option<u32>,
    shot: Option<PathBuf>,
    shot_size: (f32, f32),
    positional: Vec<String>,
    options: RenderOptions,
    dataset: Option<PathBuf>,
    quiet: bool,
}

fn parse_props(text: &str) -> Result<Json, String> {
    let text = match text.strip_prefix('@') {
        Some(file) => std::fs::read_to_string(file).map_err(|error| format!("{file}: {error}"))?,
        None => text.to_owned(),
    };
    serde_json::from_str(&text).map_err(|error| format!("props aren't JSON: {error}"))
}

fn parse(args: Vec<String>) -> Result<Parsed, String> {
    let mut parsed = Parsed {
        thumbnail: None,
        shot: None,
        shot_size: (1440., 900.),
        positional: Vec::new(),
        options: RenderOptions {
            concurrency: std::thread::available_parallelism().map_or(1, |n| n.get().min(4)),
            ..RenderOptions::default()
        },
        dataset: None,
        quiet: false,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} takes a value"));
        fn number<T: std::str::FromStr>(name: &str, text: String) -> Result<T, String> {
            text.parse()
                .map_err(|_| format!("{name} takes a number, not {text}"))
        }
        let options = &mut parsed.options;
        match arg.as_str() {
            "--codec" => {
                let name = value("--codec")?;
                options.format = Some(Format::named(&name).ok_or(format!("no codec {name}"))?);
            }
            "--fps" => options.fps = Some(number("--fps", value("--fps")?)?),
            "--scale" => options.scale = number("--scale", value("--scale")?)?,
            "--frames" => {
                let range = value("--frames")?;
                let (first, last) = range.split_once('-').unwrap_or((&range, &range));
                options.frames = Some((
                    number("--frames", first.to_owned())?,
                    number("--frames", last.to_owned())?,
                ));
            }
            "--props" => options.props = parse_props(&value("--props")?)?,
            "--dataset" => parsed.dataset = Some(value("--dataset")?.into()),
            "--transparent" => options.transparent = true,
            "--quality" => options.quality = Some(number("--quality", value("--quality")?)?),
            "--motion-blur" => options.samples = number("--motion-blur", value("--motion-blur")?)?,
            "--shutter" => options.shutter = number("--shutter", value("--shutter")?)?,
            "--mute" => options.mute = true,
            "--concurrency" | "-j" => {
                options.concurrency = number("--concurrency", value("--concurrency")?)?
            }
            "--every-nth-frame" => {
                options.every_nth = number("--every-nth-frame", value("--every-nth-frame")?)?
            }
            "--thumbnail" => parsed.thumbnail = Some(number("--thumbnail", value("--thumbnail")?)?),
            "--quiet" | "-q" => parsed.quiet = true,
            "--screenshot" => parsed.shot = Some(value("--screenshot")?.into()),
            "--preview-scale" => {
                options.scale = number("--preview-scale", value("--preview-scale")?)?
            }
            "--hold-timeout" => {
                options.hold_timeout =
                    Duration::from_secs_f64(number("--hold-timeout", value("--hold-timeout")?)?);
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            _ => parsed.positional.push(arg),
        }
    }
    Ok(parsed)
}

fn render_one(options: &RenderOptions, quiet: bool) -> Result<(), String> {
    let mut shown = 0;
    let report = zgx_video::render(options, |progress| {
        if !quiet && (progress.done == progress.total || progress.done >= shown + 30) {
            shown = progress.done;
            eprint!("\r  frame {}/{}", progress.done, progress.total);
        }
        true
    })?;
    if !quiet {
        eprintln!();
    }
    println!(
        "{}: {} frames at {}x{} in {:.2} s, {:.1} frames a second",
        report.output.display(),
        report.frames,
        report.size.0,
        report.size.1,
        report.elapsed.as_secs_f64(),
        report.rate
    );
    Ok(())
}

/// `zgx video studio <dir>`: the composition live, in a window.
#[cfg(feature = "window")]
fn studio(args: Vec<String>) -> Result<(), String> {
    let Parsed {
        positional,
        options,
        shot,
        shot_size,
        ..
    } = parse(args)?;
    let [script] = positional.as_slice() else {
        return Err(USAGE.into());
    };
    if let Some(shot) = shot {
        return zgx_video::studio::screenshot(
            script.as_ref(),
            &shot,
            shot_size,
            options.frames.map_or(0, |range| range.0),
        );
    }
    let script = std::path::absolute(script).map_err(|error| error.to_string())?;
    let app = zgx_video::studio::app_file().map_err(|error| error.to_string())?;
    let scale = options.scale;
    zgx_run::run_hosting(&app, Vec::new(), move |host| {
        match zgx_video::studio::attach(host, &script, scale) {
            Ok(studio) => {
                studio.refresh(&serde_json::json!({}));
                // It lives as long as the app does.
                std::mem::forget(studio);
            }
            Err(error) => eprintln!("zgx video: {error}"),
        }
    })
    .map_err(|error| error.to_string())
}
#[cfg(not(feature = "window"))]
fn studio(args: Vec<String>) -> Result<(), String> {
    let Parsed {
        positional,
        shot,
        shot_size,
        options,
        ..
    } = parse(args)?;
    let ([script], Some(shot)) = (positional.as_slice(), shot) else {
        return Err(
            "this zgx-video was built without a window (its `window` feature): only --screenshot"
                .into(),
        );
    };
    zgx_video::studio::screenshot(
        script.as_ref(),
        &shot,
        shot_size,
        options.frames.map_or(0, |range| range.0),
    )
}

/// `<out>` with a row's `{id}`, `{index}` and fields in it.
fn output_for(template: &str, row: &Json, index: usize) -> String {
    let mut out = template.replace("{index}", &index.to_string());
    if let Some(fields) = row.as_object() {
        for (key, value) in fields {
            let text = match value {
                Json::String(text) => text.clone(),
                other => other.to_string(),
            };
            let safe: String = text
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || "-_.".contains(c) {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            out = out.replace(&format!("{{{key}}}"), &safe);
        }
    }
    out
}

fn render(args: Vec<String>) -> Result<(), String> {
    let Parsed {
        positional,
        mut options,
        dataset,
        quiet,
        ..
    } = parse(args)?;
    let (script, composition, output) = match positional.as_slice() {
        [script, output] => (script, None, output),
        [script, composition, output] => (script, Some(composition.clone()), output),
        _ => return Err(USAGE.into()),
    };
    options.script = script.into();
    options.composition = composition;
    let Some(dataset) = dataset else {
        options.output = output.into();
        return render_one(&options, quiet);
    };
    let text = std::fs::read_to_string(&dataset)
        .map_err(|error| format!("{}: {error}", dataset.display()))?;
    let rows: Vec<Json> = serde_json::from_str(&text)
        .map_err(|error| format!("{} isn't a JSON list: {error}", dataset.display()))?;
    if !output.contains('{') {
        return Err(
            "--dataset writes one video a row: put {index} or a field's name in <out>".into(),
        );
    }
    let base = options.props.clone();
    for (index, row) in rows.iter().enumerate() {
        let mut props = base.clone();
        if let (Some(props), Some(row)) = (props.as_object_mut(), row.as_object()) {
            props.extend(row.clone());
        }
        options.props = props;
        options.output = output_for(output, row, index).into();
        render_one(&options, quiet)?;
    }
    Ok(())
}

fn still(args: Vec<String>) -> Result<(), String> {
    let Parsed {
        positional,
        mut options,
        thumbnail,
        ..
    } = parse(args)?;
    let (script, composition, frame, output) = match positional.as_slice() {
        [script, frame, output] => (script, None, frame, output),
        [script, composition, frame, output] => (script, Some(composition.clone()), frame, output),
        _ => return Err(USAGE.into()),
    };
    let frame: u64 = frame.parse().map_err(|_| format!("no frame {frame}"))?;
    options.script = script.into();
    options.composition = composition;
    let still = zgx_video::render_still(&options, frame, thumbnail)?;
    still.write_png(output.as_ref())?;
    println!("{}: {}x{}", output, still.width, still.height);
    Ok(())
}

fn list(args: Vec<String>) -> Result<(), String> {
    let Parsed {
        positional,
        options,
        ..
    } = parse(args)?;
    let [script] = positional.as_slice() else {
        return Err(USAGE.into());
    };
    let list = zgx_video::compositions(script.as_ref(), None, &options.props)?;
    for comp in &list {
        println!(
            "{}",
            json!({
                "id": comp.id, "w": comp.width, "h": comp.height, "fps": comp.fps,
                "duration": comp.duration, "props": comp.props,
            })
        );
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || matches!(args[0].as_str(), "-h" | "--help" | "help") {
        println!("{USAGE}");
        return ExitCode::from(if args.is_empty() { 2 } else { 0 });
    }
    let command = args.remove(0);
    let result = match command.as_str() {
        "render" => render(args),
        "still" => still(args),
        "list" => list(args),
        "studio" => studio(args),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("zgx video: {error}");
            ExitCode::FAILURE
        }
    }
}
