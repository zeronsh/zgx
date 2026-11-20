# zgx patch to arboard 3.6.1

Vendored from crates.io `arboard` 3.6.1 (MIT OR Apache-2.0, `LICENSE-MIT.txt`
and `LICENSE-APACHE.txt` as published) and published as `zgx-arboard` 3.6.2,
its library still `arboard`. zgx-desktop's clipboard (macOS, Windows, X11)
goes through it. The changes are marked `zgx:` in the source.

## Formats of the caller's, in one change

A copy from a web page carries types of the page's own beside its text and
HTML: a note app's citations, an editor's document. Chromium packs them into
one blob in a format of its own on each platform (`org.chromium.web-custom-data`
on macOS, `Chromium Web Custom MIME Data Format` on Windows,
`chromium/x-web-custom-data` on X11), and a page's paste reads them from it.
arboard writes text, HTML, images and files, each write replacing the last,
and reads those alone, so zgx could neither put a type of its own beside the
text nor read the blob a browser put there.

- `Set::formats(text, html, raw)` writes the text, the HTML beside it if
  given (wrapped and headed as `Set::html` does), and each `(format, bytes)`
  of `raw` as the platform names formats: a pasteboard type on macOS, a
  registered clipboard format on Windows, a target the X11 selection owner
  serves. One change of the clipboard: one `clearContents` on macOS, one
  opening and emptying on Windows, one selection's data on X11.
- `Get::raw(format)` reads a format's bytes back, from any app.

The data control protocol's clipboard (`wayland-data-control`, which zgx
doesn't enable: its own data worker serves Wayland) takes the text and HTML
of `formats` alone, and `raw` is `ClipboardNotSupported` there.

Drop the fork once arboard can write formats of the caller's beside its own
in one change and read one back.
