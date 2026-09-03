#!/bin/bash
#
# moose-patch on the handheld. **One entry point, one code path.**
#
# EmulationStation runs this as a Port; triggerhappy runs the same file on
# L2+R2. It used to take a --hotkey flag to tell those apart, which meant two
# behaviours, two sets of bugs, and a flag that could be passed wrongly. It
# works the difference out for itself now, from a fact rather than an argument:
#
#   Launched by EmulationStation, we are a descendant of `emulatorlauncher`.
#   ES has already dropped the display and is waiting on us, and it comes back
#   by itself the moment we exit. Touching it would be wrong.
#
#   Launched by triggerhappy, we are not. Nobody handed us the screen, so we
#   take it and we are the ones who must give it back.

MOOSE=/userdata/system/moose-patch
LOG=/userdata/system/logs/moose-patch.log
LOCK=/var/run/moose-patch.lock

log() { echo "$(date '+%F %T') $*" >>"$LOG"; }

# Only ever one of us.
#
# Both orderings of L2+R2 are bound, because either shoulder may go down
# first, and triggerhappy fires both rules when they arrive together. The
# first attempt compared timestamps in a file, which is a race: both copies
# read it before either wrote, both decided they were first, and two apps drew
# to DRM at once. That is "Could not queue pageflip: -16" in the log, and on
# the way out each one started its own EmulationStation — four of them, in the
# end. `mkdir` is atomic; exactly one copy can win it.
if ! mkdir "$LOCK" 2>/dev/null; then
  if [ -d "$LOCK" ] && [ -z "$(find "$LOCK" -maxdepth 0 -mmin -5 2>/dev/null)" ]; then
    rmdir "$LOCK" 2>/dev/null            # stale, from something that crashed
    mkdir "$LOCK" 2>/dev/null || exit 0
  else
    exit 0
  fi
fi
[ -n "${MOOSE_DRY_RUN:-}" ] && { log "dry run reached the lock"; rmdir "$LOCK"; exit 0; }

# Did somebody hand us the screen? Walk up our own parents and look.
handed_the_screen() {
  local pid=$PPID depth=0
  while [ "$pid" -gt 1 ] && [ "$depth" -lt 12 ]; do
    local args
    args=$(tr '\0' ' ' <"/proc/$pid/cmdline" 2>/dev/null)
    case "$args" in
      *emulatorlauncher*|*emulationstation*) return 0 ;;
    esac
    pid=$(awk '{print $4}' "/proc/$pid/stat" 2>/dev/null) || return 1
    [ -n "$pid" ] || return 1
    depth=$((depth + 1))
  done
  return 1
}

OWN_THE_SCREEN=1
handed_the_screen && OWN_THE_SCREEN=0

cleanup() {
  rmdir "$LOCK" 2>/dev/null
  # Only if we took the screen, and only if nothing is drawing. Started
  # through the profile scripts, or ES comes up with no XDG_RUNTIME_DIR,
  # cannot reach PipeWire and has no sound; with setsid, or it is our child
  # and SIGHUP takes it with us.
  [ "$OWN_THE_SCREEN" = 1 ] || return
  ps -e -o args= | grep -q '^emulationstation ' && return
  log "giving the screen back to EmulationStation"
  . /etc/profile.d/xdg.sh 2>/dev/null
  . /etc/profile.d/dbus.sh 2>/dev/null
  setsid /usr/bin/emulationstation-standalone </dev/null >/dev/null 2>&1 &
}
trap cleanup EXIT INT TERM

if [ "$OWN_THE_SCREEN" = 1 ]; then
  # A game owns the screen: L2 and R2 are the game's buttons, not ours.
  if pgrep -x retroarch >/dev/null 2>&1 || pgrep -f emulatorlauncher >/dev/null 2>&1; then
    log "L2+R2 ignored, a game is running"
    exit 0
  fi
  [ -x "$MOOSE/moose-patch" ] || { log "no app at $MOOSE/moose-patch"; exit 0; }

  log "taking the screen from EmulationStation"
  /etc/init.d/S31emulationstation stop >/dev/null 2>&1
  i=0
  while [ $i -lt 20 ]; do
    ps -e -o args= | grep -q '^emulationstation ' || break
    i=$((i + 1)); sleep 1
  done
fi

cd "$MOOSE" || exit 1

# Straight to the display. No compositor and no X server here, so SDL trying
# wayland first is a few seconds of failing before it reaches the driver that
# was always going to work.
export SDL_VIDEODRIVER="${SDL_VIDEODRIVER:-kmsdrm}"

./moose-patch "$@" >>"$LOG" 2>&1
status=$?
log "moose-patch exited $status (owned the screen: $OWN_THE_SCREEN)"

# Hand the console back clean, or the cursor and whatever text it holds show
# through whatever draws next.
printf "\033c" >/dev/tty0 2>/dev/null
exit $status
