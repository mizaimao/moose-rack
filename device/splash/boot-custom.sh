#!/bin/bash
# Things that have to be put back at every boot.
#
# The writable layer of / is a tmpfs, so everything under /usr is the stock
# image again on every start. This puts back what we want instead.
#
# Everything it reads is on **/boot**, and that is not a preference. This runs
# as S00bootcustom; /userdata is not mounted until S02resize. Anything kept
# there is invisible from here, and a hook that cannot see its own inputs
# fails by doing nothing at all — which both halves of this file did, silently,
# at every boot, until a reboot was actually checked.
#
# POSIX sh only: no `local`, no arrays, no [[ ]].
#
# MOOSE_ROOT is for the tests, which run this against a scratch directory. At
# boot it is unset and every path is the real one.

ROOT=${MOOSE_ROOT:-}
LOG=$ROOT/var/log/moose-boot.log

# In a subshell, so a /var/log that is not there yet costs a lost line and not
# a message on the boot console.
log() {
  (echo "$(date '+%F %T') $*" >>"$LOG") 2>/dev/null
}

# The KNULLI this boot is running: the first line of its version file, which
# is on the squashfs and so already there at S00.
running_knulli() {
  head -n 1 "$ROOT/usr/share/knulli/knulli.version" 2>/dev/null |
    tr -d '\r' | sed 's/^[[:space:]]*//; s/[[:space:]]*$//'
}

# Whether a switch on /boot was set on the KNULLI now running.
#
# moose-patch writes `knulli=<version>` into the switch when it is applied.
# A KNULLI update replaces /usr and keeps /boot, so without this the next boot
# would copy a driver blob or edit a script checked against the old image
# only. It skips instead, says so in the log, and the patch reads as changed
# until it is applied again on the new image.
same_knulli() {
  WROTE=$(sed -n 's/^knulli=//p' "$1" 2>/dev/null | head -n 1 | tr -d '\r')
  NOW=$(running_knulli)
  [ "$WROTE" = "$NOW" ] && return 0
  log "skipped $2: set on KNULLI '$WROTE', this is '$NOW'. Apply it again in moose-patch."
  return 1
}

# The chosen Mali driver.
#
# Marker and blobs both on /boot, and that is the whole point: this runs as
# S00bootcustom, and S02resize is what mounts /userdata. Everything this
# switcher needed used to live in /userdata, so it read nothing and did
# nothing, at every boot, silently.
apply_gpu() {
  MARKER=$ROOT/boot/moose-gpu
  [ -e "$MARKER" ] || return 0
  WANT=$(head -n 1 "$MARKER" 2>/dev/null | tr -d '\r')
  case "$WANT" in
    wayland|stock) BLOB="$ROOT/boot/moose-libmali-$WANT.so" ;;
    *) return 0 ;;
  esac
  same_knulli "$MARKER" "the $WANT graphics driver" || return 0
  [ -s "$BLOB" ] || return 0
  cp "$BLOB" "$ROOT/usr/lib/libmali.so.1.new" 2>/dev/null &&
    mv "$ROOT/usr/lib/libmali.so.1.new" "$ROOT/usr/lib/libmali.so.1" 2>/dev/null
}

# The KNULLI logo EmulationStation draws when it is loading — which is every
# game launch and every return from one. It is not a setting: ES draws
# resources/logo.png, so the only way to not see it is for that file to have
# nothing in it. Must happen before S31 starts ES.
blank_es_logo() {
  # On /boot, not /userdata: this runs as S00 and S02resize is what mounts
  # /userdata. Reading from there at S00 finds nothing, silently, and
  # EmulationStation comes back with its own logo after every reboot.
  BLANK=$ROOT/boot/moose-blank-logo.png
  DEST=$ROOT/usr/share/emulationstation/resources/logo.png
  [ -s "$BLANK" ] || return 0
  [ -e "$DEST" ] || return 0
  cp "$BLANK" "$DEST" 2>/dev/null
}

# Stop restarting evmapy for launches that have nothing for it to map.
#
# `batocera-evmapy start` kills the daemon, touches a flag and then blocks on
# inotifywait until it comes back. Measured at 0.93 s of a 3.43 s configgen
# phase, three runs, identical every time. It is a process round trip, not
# work.
#
# configgen writes a per-device .json into /var/run/evmapy *before* calling
# start, and libretro.keys declares only actions_gun1 — a lightgun combo. So a
# libretro launch with no lightgun writes no device config at all, and then
# waits 0.93 s for a daemon with nothing to do.
#
# The guard is exactly that test, and it deliberately knows nothing about
# libretro: no device config means nothing to map. The other 54 .keys files —
# flycast, amiberry, hatari, azahar, gsplus — all declare real actions_playerN
# mappings, so every standalone emulator is untouched. An unconditional stub
# would have broken all of them.
#
# Inserted rather than copied in, so a KNULLI update to this script keeps its
# own changes and only gains our line. The marker makes it idempotent.
guard_evmapy() {
  FLAG=$ROOT/boot/moose-evmapy-guard
  [ -e "$FLAG" ] || return 0
  same_knulli "$FLAG" "the evmapy guard" || return 0
  F=$ROOT/usr/bin/batocera-evmapy
  [ -s "$F" ] || return 0
  grep -q "moose-evmapy-guard" "$F" && return 0
  sed -i '/^[[:space:]]*start)/a\    ls /var/run/evmapy/*.json >/dev/null 2>&1 || exit 0 # moose-evmapy-guard' "$F" 2>/dev/null
}

case "$1" in
  start)
    apply_gpu
    blank_es_logo
    guard_evmapy
    ;;
esac
exit 0
