#!/bin/sh
# Retakes the README's pictures and its short film from the made-up sites in tests/readme_site.
# Needs Xvfb, xdotool and ffmpeg. Writes docs/assets/shots/.
set -eu
cd "$(dirname "$0")/.."
raw=.cache/readme-shots/raw
out=docs/assets/shots
rm -rf "$raw"
mkdir -p "$raw" "$out"
CANOPY_README_SHOTS="$PWD/$raw" env -u WAYLAND_DISPLAY GDK_BACKEND=x11 \
  xvfb-run -a -s '-screen 0 1440x1000x24' \
  cargo test --locked --features gui-smoke --test readme_shots

# Each picture keeps the 640 by 500 part of the window that shows its subject.
crop() {
  for theme in dark light; do
    ffmpeg -hide_banner -loglevel error -y -i "$raw/$1-$theme.png" \
      -vf "crop=640:500:$2:$3" -c:v libwebp -lossless 1 "$out/$1-$theme.webp"
  done
}
crop card 0 0
crop files 0 0
crop tree 0 0
crop forest 44 64
crop palette 300 50
crop start 300 100

ffmpeg -hide_banner -loglevel error -y -framerate 12 -i "$raw/fork/%04d.png" \
  -vf "scale=1000:-1:flags=lanczos" -c:v libwebp_anim -q:v 75 -compression_level 6 -loop 0 \
  "$out/fork.webp"
ls -l "$out"
