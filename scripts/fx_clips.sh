#!/bin/sh
# Encodes zgx-fx's clips: an H.264 MP4 and a short GIF of each, offscreen, into
# the directory given. Needs ffmpeg with libx264 (or VideoToolbox).
#   scripts/fx_clips.sh out/
set -e
out=$1
mkdir -p "$out"
cargo build --release -p zgx-fx --example fx_clip --locked
bin=${CARGO_TARGET_DIR:-target}/release/examples/fx_clip
encoder=libx264
ffmpeg -hide_banner -encoders 2>/dev/null | grep -q libx264 || encoder=h264_videotoolbox
clip() {
    name=$1
    seconds=$2
    width=$3
    size=$(SCENE=$name SECONDS=0 "$bin" 2>&1 >/dev/null | head -1)
    case $encoder in
    libx264) quality="-crf 24 -preset slow" ;;
    *) quality="-b:v 3M" ;;
    esac
    SCENE=$name SECONDS=$seconds FPS=30 "$bin" 2>/dev/null |
        ffmpeg -hide_banner -loglevel error -y -f rawvideo -pix_fmt rgba -s "$size" -r 30 -i - \
            -vf "scale=$width:-2:flags=lanczos,format=yuv420p" -c:v $encoder $quality -movflags +faststart "$out/$name.mp4"
    ffmpeg -hide_banner -loglevel error -y -i "$out/$name.mp4" \
        -vf "fps=15,scale=$4:-2:flags=lanczos,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4" \
        "$out/$name.gif"
    ls -la "$out/$name.mp4" "$out/$name.gif"
}
clip sheet 4 1610 800
for scene in lens rim_lens shockwave heat_haze glow fire fluted_glass liquid_metal wave melt; do
    clip $scene 4 1280 480
done
