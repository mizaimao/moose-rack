#!/usr/bin/env bash
#
# Fetch EmulatorJS, so the web UI can play a game in the page.
#
#   ./scripts/fetch-emulatorjs.sh          fetch and unpack if missing
#   ./scripts/fetch-emulatorjs.sh --check  say whether it is there, fetch nothing
#
# Reads assets/emulatorjs/MANIFEST.tsv. The archive is 290 MB and unpacks to
# 283 MB of cores; neither is committed. Idempotent — a correct copy is left
# alone, so this is cheap to call from a build or a deploy.
#
# Needs 7z. Debian: `apt install p7zip-full`. macOS: `brew install p7zip`.
set -euo pipefail

cd "$(dirname "$0")/.."
DIR="assets/emulatorjs"
MANIFEST="$DIR/MANIFEST.tsv"
CHECK_ONLY=false
[ "${1:-}" = "--check" ] && CHECK_ONLY=true

hash_of() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# `data/loader.js` is the one file the page actually names, so its presence is
# what "installed" means. Checking the directory would pass on a half-unpacked
# archive from an interrupted run.
if [ -f "$DIR/data/loader.js" ] && [ -d "$DIR/data/cores" ]; then
  echo "  emulatorjs present ($(ls "$DIR/data/cores"/*.data 2>/dev/null | wc -l | tr -d ' ') cores)"
  exit 0
fi
if $CHECK_ONLY; then
  echo "  emulatorjs MISSING — run scripts/fetch-emulatorjs.sh" >&2
  exit 1
fi

command -v 7z >/dev/null || { echo "7z not found (apt install p7zip-full)" >&2; exit 1; }

url=$(grep -v '^#' "$MANIFEST" | grep -v '^$' | head -1 | cut -f1)
want=$(grep -v '^#' "$MANIFEST" | grep -v '^$' | head -1 | cut -f2)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "  fetching $url"
curl -fsSL --retry 3 -o "$tmp/ejs.7z" "$url"
got=$(hash_of "$tmp/ejs.7z")
# Before unpacking, not after: a wrong archive that has already been extracted
# has put files on disk that nobody chose.
[ "$got" = "$want" ] || { echo "  hash mismatch:  want $want  got $got" >&2; exit 1; }

echo "  unpacking"
rm -rf "$DIR/data" "$DIR/LICENSE"
7z x -y -o"$tmp/x" "$tmp/ejs.7z" >/dev/null
mv "$tmp/x/data" "$DIR/data"
# The licence travels with the code, which is what GPL-3 asks.
mv "$tmp/x/LICENSE" "$DIR/LICENSE"
# EmulatorJS checks its own version against a CDN on every game start. That is
# one `fetch` to cdn.emulatorjs.org, and it only writes a line to the console --
# but this is a LAN library that has to work with the internet down, and the
# manifest above says in as many words that a CDN reference would be the only
# thing in the app phoning out while somebody is playing. Pointed at a
# page-relative name instead: it 404s, `t.ok` is false, and the check gives up
# without leaving the machine.
#
# Checked, not assumed. A vendored file that silently stopped matching would
# quietly put the call back.
before=$(grep -c 'cdn\.emulatorjs\.org/stable/data/version\.json' "$DIR/data/emulator.min.js" || true)
[ "$before" = "1" ] || { echo "  version check not where expected ($before matches) -- not patched" >&2; exit 1; }
sed -i.bak 's|https://cdn\.emulatorjs\.org/stable/data/version\.json|version.json|' "$DIR/data/emulator.min.js"
rm -f "$DIR/data/emulator.min.js.bak"

echo "  emulatorjs ready ($(ls "$DIR/data/cores"/*.data | wc -l | tr -d ' ') cores)"
