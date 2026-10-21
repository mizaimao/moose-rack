#!/usr/bin/env bash
#
# Fetch js-dos, so the web UI can play Windows 9x games in the page.
#
#   ./scripts/fetch-jsdos.sh          fetch and unpack if missing
#   ./scripts/fetch-jsdos.sh --check  say whether it is there, fetch nothing
#
# Reads assets/jsdos/MANIFEST.tsv and unpacks the package's dist/ into
# assets/jsdos/<version>/, which the service serves at /jsdos/<version>/.
# Idempotent: a version already unpacked is left alone.
set -euo pipefail

cd "$(dirname "$0")/.."
DIR="assets/jsdos"
MANIFEST="$DIR/MANIFEST.tsv"
CHECK_ONLY=false
[ "${1:-}" = "--check" ] && CHECK_ONLY=true

hash_of() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

row=$(grep -v '^#' "$MANIFEST" | grep -v '^$' | head -1)
url=$(echo "$row" | cut -f1)
want=$(echo "$row" | cut -f2)
version=$(echo "$row" | cut -f3)
# All three or nothing: an empty version would make `out` the folder holding
# the manifest, and the replace below would delete it.
[ -n "$url" ] && [ -n "$want" ] && [ -n "$version" ] || { echo "  $MANIFEST: a row needs url, sha256 and version" >&2; exit 1; }
out="$DIR/$version"

# `js-dos.js` is what the page loads and `wdosbox-x.wasm` is the emulator it
# starts, so both being there is what "installed" means. `package.json` is the
# licence record this script adds; a copy unpacked before it did is replaced.
if [ -f "$out/js-dos.js" ] && [ -f "$out/emulators/wdosbox-x.wasm" ] && [ -f "$out/package.json" ]; then
  echo "  js-dos $version present"
  exit 0
fi
if $CHECK_ONLY; then
  echo "  js-dos $version MISSING — run scripts/fetch-jsdos.sh" >&2
  exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "  fetching $url"
curl -fsSL --retry 3 -o "$tmp/js-dos.tgz" "$url"
got=$(hash_of "$tmp/js-dos.tgz")
# Before unpacking, not after: a wrong archive that has already been extracted
# has put files on disk that nobody chose.
[ "$got" = "$want" ] || { echo "  hash mismatch:  want $want  got $got" >&2; exit 1; }

tar xzf "$tmp/js-dos.tgz" -C "$tmp"
# js-dos's Dhrystone window posts its result to a server of the author's. It
# runs only when somebody opens that window, but this is a LAN library that
# keeps to the LAN, so the address is pointed at nothing: the fetch fails and
# the window says so. Checked, not assumed, as in fetch-emulatorjs.sh.
# Counted as matches, not lines: minified, two copies would be one line.
beacon='https://d5dn8hh4ivlobv6682ep.apigw.yandexcloud.net/perf/set'
count() { { grep -o -F "$1" "$tmp/package/dist/js-dos.js" || true; } | wc -l | tr -d ' '; }
before=$(count "$beacon")
[ "$before" = "1" ] || { echo "  benchmark upload not where expected ($before matches) -- not patched" >&2; exit 1; }
sed -i.bak "s|$beacon|perf-set-disabled|g" "$tmp/package/dist/js-dos.js"
rm -f "$tmp/package/dist/js-dos.js.bak"
[ "$(count "$beacon")" = "0" ] || { echo "  benchmark upload still there after patching" >&2; exit 1; }
# The package has no licence file; its package.json names the licence.
cp "$tmp/package/package.json" "$tmp/package/dist/package.json"

# Replaced whole. A folder that is there but failed the check above is a
# broken copy, and `mv` onto an existing folder would put the new one inside it.
rm -rf "$out" "$out.part"
mv "$tmp/package/dist" "$out.part"
mv "$out.part" "$out"
echo "  js-dos $version ready in $out"
