#!/usr/bin/env bash
# Builds every app icon in assets/appicons/ into the shapes each OS wants, and
# copies the default one into src-tauri/icons/ where the bundler looks.
#
# Adding an icon is dropping a 1024x1024 PNG into assets/appicons/ and naming it
# in src/appicon.rs. Nothing here is per-icon.
#
# macOS masks nothing: whatever is in the .icns is drawn as-is. So the .icns is
# cut to Apple's own grid — a rounded square filling 80.5% of the canvas, with
# the rest transparent. That number is not a guess: every icon in
# /System/Applications measures 206 opaque pixels across a 256 canvas. Fill the
# canvas instead and the app sits noticeably larger than its neighbours; the
# earlier square-cornered icon did both, being full-bleed and unrounded.
#
# Windows and Linux draw their own frame, so they get the plain square.
set -euo pipefail
cd "$(dirname "$0")/.."

DEFAULT="${1:-arcade}"
SRC=assets/appicons
OUT=assets/appicons/built

command -v magick >/dev/null || { echo "needs ImageMagick (brew install imagemagick)"; exit 1; }

# Apple's grid, both measured from /System/Applications rather than published:
# the rounded square covers 80.5% of the canvas, and its corner radius is 22.5%
# of the square (not of the canvas).
BODY_PCT=80.5
RADIUS_PCT=22.5

# The drop shadow, also measured: 14px of blur at 45% black, sitting 8px below
# the body on a 1024 canvas.
SHADOW_SIGMA=14
SHADOW_DROP=8
SHADOW_ALPHA=0.45

# How much to zoom into a source before it becomes the tile, per icon.
#
# Artwork is framed for its own sake, not for a 40-pixel square: the arcade
# render leaves a wide margin of background around the cabinet, which reads as
# a small icon however big the tile is. This crops that margin away. 100 means
# use the source as it came.
zoom_for() {
  case "$1" in
    arcade) echo 100 ;;
    *)      echo 100 ;;
  esac
}

rm -rf "$OUT"
mkdir -p "$OUT"

for src in "$SRC"/*.png; do
  id=$(basename "$src" .png)
  work="$OUT/$id"
  mkdir -p "$work"

  # Square, 1024, whatever came in, cropped to this icon's framing.
  # PNG32: forces RGBA. Tauri's bundler refuses any icon that is not RGBA,
  # and a plain resize of an opaque JPEG-ish PNG comes out RGB.
  z=$(zoom_for "$id")
  side=$(python3 -c "print(round(1024 * $z / 100))")
  magick "$src" -resize "${side}x${side}^" -gravity center -extent 1024x1024 \
    PNG32:"$work/full.png"

  # The macOS body: the rounded square, sized to Apple's grid and centred on a
  # transparent 1024 canvas.
  body=$(python3 -c "print(round(1024 * $BODY_PCT / 100))")
  r=$(python3 -c "print(round($body * $RADIUS_PCT / 100))")
  # -fill white matters: the default fill is black, which composites to a
  # fully transparent icon and looks like the build silently produced nothing.
  magick -size "${body}x${body}" xc:black -fill white \
    -draw "roundrectangle 0,0,$((body-1)),$((body-1)),$r,$r" "$work/mask.png"
  magick "$work/full.png" -resize "${body}x${body}" "$work/mask.png" \
    -alpha off -compose CopyOpacity -composite PNG32:"$work/body.png"
  # The shadow Apple's icons carry and ours did not, which is most of why this
  # one read as smaller than its neighbours: a flat dark tile on a dark Dock has
  # no edge, so it recedes. Measured off /System/Applications rather than
  # invented — Music and Calendar are both 206 opaque pixels on a 256 canvas
  # and 220 once the shadow is counted, which at 1024 is a body of 824 centred
  # and a total extent of 880 sitting 8px lower. Sigma 14 and 45% reproduce
  # that to the pixel.
  #
  # The body is composited at the exact centre afterwards, so the shadow can
  # never shift it: an icon that is 2px off-centre is one that looks wrong in a
  # Dock and gives no clue why.
  off=$((SHADOW_DROP))
  magick -size 1024x1024 xc:none "$work/body.png" -geometry "+$(( (1024-body)/2 ))+$(( (1024-body)/2 + off ))" \
    -composite -alpha extract -blur "0x${SHADOW_SIGMA}" PNG32:"$work/shadow-alpha.png"
  magick -size 1024x1024 xc:black "$work/shadow-alpha.png" -alpha off \
    -compose CopyOpacity -composite \
    -channel A -evaluate multiply "$SHADOW_ALPHA" +channel PNG32:"$work/shadow.png"
  magick "$work/shadow.png" "$work/body.png" \
    -geometry "+$(( (1024-body)/2 ))+$(( (1024-body)/2 ))" -composite \
    PNG32:"$work/rounded.png"

  # macOS wants ten sizes in an .iconset; iconutil refuses a folder missing any.
  iconset="$work/$id.iconset"
  mkdir -p "$iconset"
  for s in 16 32 128 256 512; do
    magick "$work/rounded.png" -resize "${s}x${s}"     PNG32:"$iconset/icon_${s}x${s}.png"
    magick "$work/rounded.png" -resize "$((s*2))x$((s*2))" PNG32:"$iconset/icon_${s}x${s}@2x.png"
  done
  if command -v iconutil >/dev/null; then
    iconutil -c icns "$iconset" -o "$work/$id.icns"
  else
    echo "no iconutil (not macOS) — skipping $id.icns"
  fi

  # Windows and Linux: the square, unrounded.
  magick "$work/full.png" -define icon:auto-resize=256,128,64,48,32,16 "$work/$id.ico"
  for s in 32 128 256 512; do
    magick "$work/full.png" -resize "${s}x${s}" PNG32:"$work/${s}x${s}.png"
  done
  magick "$work/full.png" -resize 256x256 PNG32:"$work/128x128@2x.png"

  # The preview the Settings picker draws.
  magick "$work/rounded.png" -resize 256x256 PNG32:"$work/preview.png"

  rm -f "$work/mask.png" "$work/body.png" "$work/shadow.png" "$work/shadow-alpha.png"
  echo "built $id"
done

# The default is what the bundler compiles in, so a fresh install starts there.
d="$OUT/$DEFAULT"
[ -d "$d" ] || { echo "no such icon: $DEFAULT"; exit 1; }
cp "$d/$DEFAULT.icns" src-tauri/icons/icon.icns
cp "$d/$DEFAULT.ico"  src-tauri/icons/icon.ico
cp "$d/full.png"      src-tauri/icons/icon.png
cp "$d/32x32.png"     src-tauri/icons/32x32.png
cp "$d/128x128.png"   src-tauri/icons/128x128.png
cp "$d/128x128@2x.png" src-tauri/icons/128x128@2x.png
rm -rf src-tauri/icons/icon.iconset src-tauri/icons/macos.iconset
cp -R "$d/$DEFAULT.iconset" src-tauri/icons/icon.iconset

# Everything else was scaffolding for the two files above. The bundler globs
# this folder into the app, so what is left here is what every user downloads:
# the macOS icon to swap in, the square PNG the window icon is set from, and
# the picture the picker draws. Nothing else earns its bytes.
for work in "$OUT"/*/; do
  id=$(basename "$work")
  find "$work" -mindepth 1 \
    ! -name "$id.icns" ! -name "256x256.png" ! -name "preview.png" \
    -delete 2>/dev/null || true
done

echo "default is $DEFAULT"
