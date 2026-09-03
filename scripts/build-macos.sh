#!/usr/bin/env bash
#
# Build Moose Rack.app locally on macOS.
#
# macOS is deliberately absent from the release workflow. A CI-built bundle is
# unsigned, so it arrives quarantined: the user has to mount a disk image, drag
# the app out, then clear com.apple.quarantine before it will open. Building on
# the machine that runs it skips all of that — a locally-produced bundle carries
# no quarantine attribute at all.
#
#   ./scripts/build-macos.sh            build, place the bundle in the repo root
#   ./scripts/build-macos.sh --link     also symlink it into ~/Applications/MooseStack
#   ./scripts/build-macos.sh --cli      build only the command-line binary
#
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"

# The fonts the app ships. Pinned and hashed in assets/fonts/MANIFEST.tsv and
# never committed, so a fresh checkout has none until this runs. Cheap when
# they are already there.
./scripts/fetch-fonts.sh
APP="Moose Rack.app"
LINK_DIR="$HOME/Applications/MooseStack"

link=0
cli_only=0
for arg in "$@"; do
  case "$arg" in
    --link) link=1 ;;
    --cli)  cli_only=1 ;;
    -h|--help) sed -n '3,15p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

[ "$(uname -s)" = "Darwin" ] || { echo "this script is macOS-only" >&2; exit 1; }

# Apple silicon and Intel need different targets; ask the machine rather than
# assuming, so this works on both.
case "$(uname -m)" in
  arm64) TARGET="aarch64-apple-darwin" ;;
  x86_64) TARGET="x86_64-apple-darwin" ;;
  *) echo "unexpected architecture: $(uname -m)" >&2; exit 1 ;;
esac

echo "==> building for $TARGET"
cargo build --release --locked --target "$TARGET" --bin moose-rack
echo "    cli: target/$TARGET/release/moose-rack"

if [ "$cli_only" -eq 1 ]; then exit 0; fi

# tauri.conf.json lists several bundle targets; only 'app' is meaningful here.
# Skipping the dmg keeps the build quick and avoids producing the very artifact
# whose handling this script exists to avoid.
echo "==> building the app bundle"
npx tauri build --target "$TARGET" --bundles app

BUILT="target/$TARGET/release/bundle/macos/$APP"
[ -d "$BUILT" ] || { echo "expected a bundle at $BUILT" >&2; exit 1; }

# The repo root copy is what gets launched during development, so replace it
# wholesale rather than merging into a stale bundle.
rm -rf "${ROOT:?}/$APP"
cp -R "$BUILT" "$ROOT/$APP"
echo "    app: $ROOT/$APP"

if [ "$link" -eq 1 ]; then
  mkdir -p "$LINK_DIR"
  ln -sfn "$ROOT/$APP" "$LINK_DIR/$APP"
  echo "    linked: $LINK_DIR/$APP"
fi

# A locally built bundle is not quarantined, but a previous copy downloaded from
# a release might have left the attribute on the path. Clearing it is harmless
# when there is nothing to clear.
xattr -dr com.apple.quarantine "$ROOT/$APP" 2>/dev/null || true

echo "==> done"
