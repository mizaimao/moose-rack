#!/usr/bin/env bash
#
# Seed the server's ES-DE library from the SSD.
#
# The new service is developed against `/home/frank/moose-library` on dev.lan,
# which follows the ES-DE layout and holds seven systems rather than the whole
# 1.8 TB: about 10 GB of ROMs and 71 GB of artwork. Small enough to copy in an
# hour, representative enough to build against. RomM stays up and untouched
# until the service replaces it.
#
#     ROMs/<system>/
#     ES-DE/gamelists/<system>/gamelist.xml
#     ES-DE/downloaded_media/<system>/<type>/
#
# Note the source layout is not the destination layout: the SSD is a *portable*
# ES-DE install, so its artwork lives under `support/` while its gamelists do
# not. The destination is the ordinary arrangement, which is what `Layout::new`
# derives without an override.
#
# macOS ships openrsync, which has neither --info=progress2 nor
# --no-inc-recursive. Plain -a --partial only; re-running resumes.
set -uo pipefail
SYS=(sfc gbc gba gb snes nes famicom)
SSD=/Volumes/Retro
DEST=dev.lan:/home/frank/moose-library
for s in "${SYS[@]}"; do
  for pair in "Roms/$s ROMs/$s" "ES-DE/gamelists/$s ES-DE/gamelists/$s" "ES-DE/support/downloaded_media/$s ES-DE/downloaded_media/$s"; do
    set -- $pair
    src="$SSD/$1/" ; dst="$DEST/$2/"
    echo "=== $s : $2 ($(du -sh "$SSD/$1" 2>/dev/null | cut -f1)) $(date +%H:%M:%S)"
    /usr/bin/rsync -a --partial "$src" "$dst" && echo "    ok" || echo "    FAILED"
  done
done
echo "=== done $(date +%H:%M:%S) ==="
