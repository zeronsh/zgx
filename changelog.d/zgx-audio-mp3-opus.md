---
section: added
---
- **MP3 and Ogg Opus** (zgx-audio, `mp3` and `opus` features, both off by default): MP3 through nanomp3 (pure Rust, MIT/Apache, +65 KiB) and Ogg Opus through opus-pure (pure Rust, BSD-3-Clause, +307 KiB), each loaded whole or streamed. Both are gapless: MP3 where its LAME or Info tag records its delay and padding, Opus by its pre-skip and end trim.
  - Conformance: both are held to the formats' public vectors, fetched only when asked for (`--test conformance -- --ignored`). The eleven ISO Layer III compliance streams decode within 1 LSB of their reference, and every packet of the twelve RFC 8251 Opus vectors leaves the range coder where the vector says.
  - Hostile files: empty, cut short at every length, absurd headers, or bytes flipped at random, every format comes back without a panic, a hang, or more than 64 MiB live.
  - Limits: an MP3 that switches between mono and stereo ends at its first switch, and Opus plays one or two channels.
