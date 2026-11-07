# zgx-video media

"Every frame", the showcase of `zgx-video` (`crates/zgx-video/examples/showcase`), rendered headless at 21b129ac (main 6a4c587f with the showcase's update) with `zgx video render . every-frame.mp4 -j 4 --motion-blur 4`: 1920×1080, 60 fps, H.264 and AAC, 33 s. Its music is synthesized by the example's `make_score.py`.

It uses captions from `captions.srt` with each word lit as it's said, bars drawn from its own score (`video.audio_data`), rings looping on the beat (`video.loop`) and a strip of earlier acts each held on one frame (`video.freeze`).

- `every-frame.mp4`: the film.
- `poster.png`: `zgx video still . 1890 poster.png`; `thumbnail.png`: the same with `--thumbnail 480`.
- `studio.png`: `zgx video studio . --screenshot studio.png --frames 1440-1440`, the studio's player at 24 s.
- `shaders.gif`, `glass.gif`, `finale.gif`: three acts, for pull requests.
