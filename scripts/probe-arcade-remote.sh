#!/usr/bin/env bash
#
# Probe every arcade romset on the server, headless and in parallel.
#
# The server is the right place for this. macOS has no headless mode — Cocoa
# creates RetroArch's window before the video driver is chosen, so probing 2,500
# games opens 2,500 windows and the machine is unusable for hours. A Debian box
# with no DISPLAY runs the same RetroArch, same version, and shows nothing.
#
# Each worker writes to its own log. That is not tidiness: the verdict is read
# *out of the log*, so two workers sharing one would each read the other's
# result and the run would produce confident nonsense.
#
# No verdict is decided here. This collects the lines that matter; the Rust side
# classifies them, so there is one implementation of "did it run" rather than
# two that drift apart.
#
#   scripts/probe-arcade-remote.sh [jobs]
#
set -euo pipefail

HOST="dev.lan"
JOBS="${1:-4}"

/usr/bin/ssh -o BatchMode=yes "$HOST" "JOBS=$JOBS bash -s" <<'REMOTE'
set -euo pipefail
ROMS="/home/frank/romm/assets/roms/arcade"
CORES="/home/frank/.config/retroarch/cores"
OUT="/home/frank/probe-results.tsv"

mkdir -p /tmp/probe
# input_driver matters as much as video. With only video nulled, RetroArch
# picks an input driver from the video driver, finds none, and exits with
# "Cannot initialize input driver" — *after* the core has loaded the game and
# reported its geometry. Every MAME game then looks like a refusal when what
# actually failed was the frontend.
printf 'config_save_on_exit = "false"\nvideo_driver = "null"\naudio_driver = "null"\nmenu_driver = "null"\ninput_driver = "null"\njoypad_driver = "null"\npause_nonactive = "false"\n' > /tmp/probe/headless.cfg

run_one() {
  local rom="$1" core="fbneo"
  # One log per *game*, not per worker slot. Slots were reused as soon as a
  # worker freed up, so a fast job could start writing over a slower one still
  # holding the same slot — and since the verdict is read out of the log, 267
  # of 2,503 games came back carrying another game's result. A name that cannot
  # collide is the only version of this that is safe.
  local log="/tmp/probe/$(basename "$rom" .zip).log"
  rm -f "$log"
  timeout -k 5 120 retroarch --appendconfig=/tmp/probe/headless.cfg \
      --max-frames=60 --verbose --log-file="$log" \
      -L "$CORES/${core}_libretro.so" "$rom" >/dev/null 2>&1 || true

  # -k above sends SIGKILL once SIGTERM is ignored. Without it three games hung
  # for three hours: RetroArch does not always honour a TERM.
  #
  # Only the lines the verdict depends on. A full log is hundreds of lines of
  # driver chatter, and 2,500 of those is a gigabyte of noise to carry back.
  # Newlines become 0x1f so one game stays one record; the reader puts them back.
  local keep
  keep=$(grep -aE "successfully started|missing files|Missing files|Required files are missing|NOT FOUND|WRONG CHECKSUM|Bioses aren|Failed to load content|Failed to (open|load) libretro|ran for a total of" "$log" 2>/dev/null | head -20 | tr '\n' '\037' || true)
  rm -f "$log"
  printf '%s\t%s\t%s\n' "$(basename "$rom")" "$core" "$keep"
}
export -f run_one
export CORES

: > "$OUT"
ls "$ROMS"/*.zip | xargs -P "$JOBS" -I{} bash -c 'run_one "{}"' >> "$OUT"

# Self-check. FBNeo names the driver it started, and that name must be the game
# that was asked for. Any mismatch means results crossed between workers, which
# is a silent failure that reads as a perfectly good report.
python3 - "$OUT" <<'CHK'
import sys
bad = shared = 0
seen = {}
rows = [l.rstrip("\n").split("\t") for l in open(sys.argv[1])]
for r in rows:
    if len(r) < 3: continue
    rom, _, log = r
    if log:
        if log in seen and seen[log] != rom: shared += 1
        seen[log] = rom
    if "Driver " in log:
        drv = log.split("Driver ", 1)[1].split(" ", 1)[0]
        if drv != rom[:-4]: bad += 1
print(f"records: {len(rows)}")
print(f"cross-contaminated: {bad} wrong driver, {shared} shared log text")
if bad or shared:
    print("RESULTS NOT TRUSTWORTHY")
CHK
REMOTE
