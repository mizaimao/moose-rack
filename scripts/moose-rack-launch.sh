#!/bin/bash
#
# Moose Rack on the handheld. One file, two ways in:
#
#   moose-rack-launch.sh            EmulationStation started us, as a Port. ES has
#                             already dropped the display and is waiting on us.
#   moose-rack-launch.sh --hotkey   triggerhappy started us, on L2+R2. Nobody dropped
#                             anything, so we have to stop ES ourselves.
#
# The difference is only who owns the screen. ES holds DRM master and lets go
# for children it launches itself; nothing launched from outside gets that, so
# the hotkey path has to stop ES and put it back afterwards.

MOOSE_RACK=/userdata/system/moose-rack
LOG=/userdata/system/logs/moose-hotkey.log
STAMP=/var/run/moose-hotkey.stamp
HOTKEY=0
[ "${1:-}" = "--hotkey" ] && { HOTKEY=1; shift; }

if [ "$HOTKEY" = 1 ]; then
  log() { echo "$(date '+%F %T') $*" >>"$LOG"; }

  # Both orderings of the combo are bound, so without this it fires twice.
  now=$(cut -d' ' -f1 /proc/uptime)
  last=$(cat "$STAMP" 2>/dev/null || echo 0)
  [ "$(awk -v n="$now" -v l="$last" 'BEGIN{print (n-l < 3)}')" = "1" ] && exit 0
  echo "$now" >"$STAMP"

  # A game owns the screen: those two buttons are the game's, not ours.
  if pgrep -x retroarch >/dev/null 2>&1 || pgrep -f emulatorlauncher >/dev/null 2>&1; then
    log "ignored, a game is running"; exit 0
  fi
  [ -x "$MOOSE_RACK/moose-sdl" ] || { log "no app at $MOOSE_RACK/moose-sdl"; exit 0; }

  # Whatever happens below, ES comes back — a crash here would otherwise leave
  # a black screen with no way to the menu. Through the init script, because
  # started by hand ES comes up without XDG_RUNTIME_DIR and has no sound.
  trap '/etc/init.d/S31emulationstation start >/dev/null 2>&1' EXIT INT TERM

  log "L2+R2, stopping EmulationStation"
  /etc/init.d/S31emulationstation stop >/dev/null 2>&1
  i=0
  while [ $i -lt 20 ]; do
    ps -e -o args | grep -q '^emulationstation ' || break
    i=$((i + 1)); sleep 1
  done
fi

# The working directory matters. The app looks for `data/esde-core-map.json`
# and its own `moose-sdl.toml` relative to where it was started, so a launcher
# that does not cd finds neither: an empty library and default settings, with
# nothing on screen to say why.
cd "$MOOSE_RACK" || exit 1

# Plain redirection, not `tee`. Process substitution starts a `tee` that
# inherits our standard output and outlives the app, so whatever is waiting on
# this script goes on waiting after the app has gone.
exec >./log.txt 2>&1
echo "--- $(date) ---"

# Straight to the display. No compositor and no X server here, so SDL trying
# wayland first is a few seconds of failing before it reaches the driver that
# was always going to work.
export SDL_VIDEODRIVER="${SDL_VIDEODRIVER:-kmsdrm}"

./moose-sdl "$@"
status=$?
echo "--- exit $status ---"

# Hand the console back clean, or the cursor and whatever text it holds show
# through whatever draws next.
printf "\033c" >/dev/tty0 2>/dev/null
exit $status
