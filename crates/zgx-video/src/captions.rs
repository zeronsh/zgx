//! Captions from SubRip (`.srt`) and WebVTT (`.vtt`) files: each cue's
//! start, end and text, for a video to show at its time.

/// A caption shown from `start` to `end` seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    /// When it shows, in seconds.
    pub start: f64,
    /// When it goes.
    pub end: f64,
    /// What it says, its lines joined by `\n`, tags taken out.
    pub text: String,
}

/// `00:01:02,500`, `01:02.500` or `00:01:02.500` as seconds.
fn timestamp(text: &str) -> Option<f64> {
    let text = text.trim().replace(',', ".");
    let parts: Vec<&str> = text.split(':').collect();
    let (h, m, s) = match parts.as_slice() {
        [h, m, s] => (
            h.parse::<f64>().ok()?,
            m.parse::<f64>().ok()?,
            s.parse::<f64>().ok()?,
        ),
        [m, s] => (0., m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        _ => return None,
    };
    Some(h * 3600. + m * 60. + s)
}

/// Text with `<i>`, `<b>`, `<c.x>`, `<00:00:01.000>` and other tags taken out.
fn untagged(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut depth = 0;
    for c in line.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
}

/// The cues of an SRT or WebVTT file's text, in order of their start.
pub fn parse(text: &str) -> Vec<Cue> {
    let text = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut cues = Vec::new();
    for block in text.split("\n\n") {
        let mut lines = block
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .peekable();
        // An index (SRT) or an identifier (VTT) may come before the timing.
        let timing = loop {
            let Some(line) = lines.next() else { break None };
            if line.contains("-->") {
                break Some(line);
            }
        };
        let Some(timing) = timing else { continue };
        let Some((from, rest)) = timing.split_once("-->") else {
            continue;
        };
        // VTT settings follow the end: `00:02.000 align:start`.
        let to = rest.split_whitespace().next().unwrap_or_default();
        let (Some(start), Some(end)) = (timestamp(from), timestamp(to)) else {
            continue;
        };
        let text: Vec<String> = lines.map(untagged).collect();
        if !text.is_empty() {
            cues.push(Cue {
                start,
                end,
                text: text.join("\n"),
            });
        }
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    cues
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subrip_reads_its_cues() {
        let cues = parse(
            "\u{feff}1\r\n00:00:01,000 --> 00:00:02,500\r\nHello <i>there</i>\r\n\r\n2\r\n00:00:03,000 --> 00:01:04,250\r\nTwo\r\nlines\r\n",
        );
        assert_eq!(
            cues,
            [
                Cue {
                    start: 1.,
                    end: 2.5,
                    text: "Hello there".into()
                },
                Cue {
                    start: 3.,
                    end: 64.25,
                    text: "Two\nlines".into()
                },
            ]
        );
    }
    #[test]
    fn webvtt_reads_its_cues_and_skips_its_header_and_settings() {
        let cues = parse(
            "WEBVTT\n\nNOTE a comment\n\nintro\n00:01.000 --> 00:02.000 align:start\n<v Ann>Fish &amp; chips\n\n01:00:00.000 --> 01:00:01.500\n<c.loud>Late</c>\n",
        );
        assert_eq!(cues.len(), 2);
        assert_eq!(
            cues[0],
            Cue {
                start: 1.,
                end: 2.,
                text: "Fish & chips".into()
            }
        );
        assert_eq!((cues[1].start, cues[1].text.as_str()), (3600., "Late"));
    }
}
