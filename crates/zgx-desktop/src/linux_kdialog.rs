//! A file chooser where no portal answers on a KDE system without zenity:
//! kdialog's, which is KDE's own (KFileWidget), as zenity's is GTK's. Its
//! process is run, attached and cancelled as zenity's is (`linux_zenity::run`).
use crate::{FileDialogOptions, FileDialogResult, file_dialog::Kind};
use std::path::PathBuf;
use winit::window::Window;

/// Whether kdialog is on the `PATH`.
pub(crate) fn available() -> bool {
    crate::linux_zenity::on_path("kdialog")
}

/// Whether the session is KDE's, where kdialog's chooser is the native one and
/// comes before zenity's.
pub(crate) fn preferred() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|desktops| desktops.split(':').any(|d| d.eq_ignore_ascii_case("KDE")))
}

/// kdialog's arguments for a `kind` request with `options`, attached to the
/// X11 window `xid`. It has no several-folder chooser: Folders picks one.
pub(crate) fn arguments(kind: Kind, options: &FileDialogOptions, xid: Option<u64>) -> Vec<String> {
    let mut args = Vec::new();
    if !options.title.is_empty() {
        args.extend(["--title".to_owned(), options.title.clone()]);
    }
    if let Some(xid) = xid {
        args.extend(["--attach".to_owned(), xid.to_string()]);
    }
    if matches!(kind, Kind::OpenFiles) {
        args.extend(["--multiple".to_owned(), "--separate-output".to_owned()]);
    }
    args.push(
        match kind {
            Kind::OpenFile | Kind::OpenFiles => "--getopenfilename",
            Kind::Folder | Kind::Folders => "--getexistingdirectory",
            Kind::Save => "--getsavefilename",
        }
        .to_owned(),
    );
    // The start is positional, before the filter: a folder, or a file in one.
    args.push(match (&options.directory, &options.file_name) {
        (Some(dir), Some(name)) => dir.join(name).display().to_string(),
        (Some(dir), None) => dir.display().to_string(),
        (None, Some(name)) => name.clone(),
        (None, None) => ".".to_owned(),
    });
    if !matches!(kind, Kind::Folder | Kind::Folders) && !options.filters.is_empty() {
        // KDE's filter string: a "patterns|label" line per filter.
        let lines: Vec<String> = options
            .filters
            .iter()
            .map(|filter| {
                let patterns: Vec<String> = filter
                    .extensions
                    .iter()
                    .map(|e| {
                        if e == "*" {
                            "*".into()
                        } else {
                            format!("*.{e}")
                        }
                    })
                    .collect();
                format!("{}|{}", patterns.join(" "), filter.name)
            })
            .collect();
        args.push(lines.join("\n"));
    }
    args
}

/// What kdialog printed: a path a line.
pub(crate) fn paths(output: &str) -> Vec<PathBuf> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Show kdialog's chooser for `parent` and wait for it, or for `cancel`.
pub(crate) fn select(
    kind: Kind,
    options: &FileDialogOptions,
    parent: &Window,
    cancel: &crate::file_dialog::Cancel,
) -> FileDialogResult<Vec<PathBuf>> {
    let args = arguments(kind, options, crate::linux_zenity::xid(parent));
    Ok(crate::linux_zenity::run("kdialog", args, cancel)?.map(|output| paths(&output)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileFilter;

    fn options() -> FileDialogOptions {
        FileDialogOptions {
            title: "Open fixture".into(),
            directory: Some("/tmp/docs".into()),
            file_name: None,
            filters: vec![
                FileFilter {
                    name: "Text".into(),
                    extensions: vec!["txt".into(), "md".into()],
                },
                FileFilter {
                    name: "All files".into(),
                    extensions: vec!["*".into()],
                },
            ],
        }
    }

    #[test]
    fn requests_become_kdialog_arguments() {
        assert_eq!(
            arguments(Kind::OpenFiles, &options(), Some(42)),
            [
                "--title",
                "Open fixture",
                "--attach",
                "42",
                "--multiple",
                "--separate-output",
                "--getopenfilename",
                "/tmp/docs",
                "*.txt *.md|Text\n*|All files",
            ]
        );
        let save = FileDialogOptions {
            file_name: Some("notes.txt".into()),
            ..options()
        };
        let args = arguments(Kind::Save, &save, None);
        assert_eq!(&args[2..4], ["--getsavefilename", "/tmp/docs/notes.txt"]);
        let folder = arguments(
            Kind::Folders,
            &FileDialogOptions {
                title: String::new(),
                ..options()
            },
            None,
        );
        assert_eq!(
            folder,
            ["--getexistingdirectory", "/tmp/docs"],
            "no filter, no several folders"
        );
        let bare = FileDialogOptions {
            title: String::new(),
            directory: None,
            file_name: None,
            filters: vec![],
        };
        assert_eq!(
            arguments(Kind::OpenFile, &bare, None),
            ["--getopenfilename", "."]
        );
    }

    #[test]
    fn kdialogs_output_reads_as_paths() {
        assert_eq!(paths("/a b/c.txt\n"), [PathBuf::from("/a b/c.txt")]);
        assert_eq!(
            paths("/x\n/y z\n"),
            [PathBuf::from("/x"), PathBuf::from("/y z")]
        );
        assert!(paths("\n").is_empty());
    }
}
