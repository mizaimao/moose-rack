#!/usr/bin/env bash
#
# Fetch the fonts the app ships. Reads assets/fonts/MANIFEST.tsv.
#
# The files are never committed — they are a build input, pinned by URL and
# verified by hash, and a repository is a poor place to keep two megabytes of
# somebody else's binary. What *is* committed is the manifest, so anybody can
# get byte-for-byte the same fonts, and LICENSES.md, because the SIL Open Font
# License requires the licence to travel with the font.
#
#   ./scripts/fetch-fonts.sh             fetch anything missing or wrong
#   ./scripts/fetch-fonts.sh --check     say what is missing, fetch nothing
#   ./scripts/fetch-fonts.sh --with-cjk  add the CJK set (50 MB)
#
# CJK is not in the default set because both targets already have it — the
# handheld installs fonts-noto-cjk, macOS ships PingFang and Hiragino — and it
# is fifty megabytes against two. A Linux desktop without the Debian package,
# or Windows, wants --with-cjk.
#
# Idempotent: a file already present with the right hash is left alone, so this
# is cheap to call from every build.
set -euo pipefail

cd "$(dirname "$0")/.."
DIR="assets/fonts"
MANIFEST="$DIR/MANIFEST.tsv"
CHECK_ONLY=false
WITH_CJK=false
for arg in "$@"; do
  case "$arg" in
    --check) CHECK_ONLY=true ;;
    --with-cjk) WITH_CJK=true ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

[ -f "$MANIFEST" ] || { echo "no $MANIFEST" >&2; exit 1; }

# sha256, wherever we are. macOS has shasum, Debian has sha256sum.
hash_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

missing=0
fetched=0
section=default
while IFS=$'\t' read -r name want size url; do
  case "$name" in
    ''|'#'*) continue ;;
    '[cjk]') section=cjk; continue ;;
  esac
  # The CJK set is skipped unless asked for. Its hashes are left as `-` in the
  # manifest: pinning them would mean this file grew fifty megabytes of
  # provenance for something almost nobody fetches, and the URLs are already
  # pinned to a family and a name.
  if [ "$section" = cjk ] && ! $WITH_CJK; then
    continue
  fi
  out="$DIR/$name"

  if [ -f "$out" ] && { [ "$want" = "-" ] || [ "$(hash_of "$out")" = "$want" ]; }; then
    continue
  fi

  if $CHECK_ONLY; then
    echo "missing or stale: $name"
    missing=$((missing + 1))
    continue
  fi

  echo "fetching $name (${size} bytes)"
  tmp="$out.part"
  curl -sfL --retry 3 "$url" -o "$tmp"

  got=$(hash_of "$tmp")
  if [ "$want" != "-" ] && [ "$got" != "$want" ]; then
    rm -f "$tmp"
    # Not a warning. A font that is not the font the manifest names is either a
    # broken download or a different file at the same URL, and both mean the
    # build must stop rather than ship something nobody pinned.
    echo "ERROR: $name hashed $got, manifest says $want" >&2
    exit 1
  fi
  mv "$tmp" "$out"
  fetched=$((fetched + 1))
done < "$MANIFEST"

if $CHECK_ONLY; then
  [ "$missing" -eq 0 ] && echo "all fonts present" || exit 1
else
  echo "==> fonts ready ($fetched fetched)"
fi
