# zgx-video

Videos written as zgx interfaces, in Luau or Rust: rendered offscreen a frame at a time on a virtual clock that every animation, timer and shader follows, to H.264, HEVC, ProRes 4444 with alpha, PNGs or a GIF, with a studio to preview them. An app that doesn't depend on this crate links none of it.

It isn't on crates.io yet: from a checkout of the repository, `cargo install --path crates/zgx-video`. An app using `zgx_video` as a library takes every zgx crate from the same revision of the repository.

```sh
zgx video render examples/showcase film.mp4 --motion-blur 4
zgx video studio examples/showcase
```

See [docs/video.md](../../docs/video.md). `examples/hello` is the smallest composition; `examples/showcase` is a 33-second film (run its `make_score.py` first for its music).
