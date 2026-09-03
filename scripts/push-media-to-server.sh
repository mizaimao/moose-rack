#!/usr/bin/env bash
#
# Push this machine's ES-DE artwork to the RomM server.
#
# Two steps, because `resources/esde-media` on the server is owned by root and
# sudo there wants a password. Step one needs neither: it lands in your own home
# directory over SSH. Step two moves it into place as root, and is left for you
# to run.
#
#   scripts/push-media-to-server.sh            push to staging
#   scripts/push-media-to-server.sh --wait     wait for a running card import first
#
set -euo pipefail

cd "$(dirname "$0")/.."

HOST="dev.lan"
SRC="library/downloaded_media/"
STAGING="/home/frank/esde-staging"
LIVE="/home/frank/romm/resources/esde-media"

if [ "${1:-}" = "--wait" ]; then
  # The card import may still be running. Wait it out rather than pushing a
  # half-copied tree and having to reconcile the difference later.
  while pgrep -f "import-esde-media.py" > /dev/null; do sleep 30; done
  echo "==> card import finished"
fi

echo "==> pushing $SRC to $HOST:$STAGING"
/usr/bin/ssh -o BatchMode=yes "$HOST" "mkdir -p $STAGING"

# --ignore-existing rather than --update: every file here is a copy of a file on
# the card, so anything already on the server is already the same bytes, and
# re-sending 90 GB to prove it is not worth the wire time.
#
# The dot-files are this app's own bookkeeping — the negative-lookup index, the
# scraped manifest, the cache-cleared marker. They are about this machine and
# mean nothing on the server.
# Plain flags only. macOS ships openrsync, which speaks protocol 29 and has
# none of rsync 3's niceties -- --info=progress2 is not a flag it knows, and it
# answers an unknown flag with a usage dump rather than a message saying so.
rsync -a --ignore-existing \
  --exclude '.*' \
  --exclude 'covers_thumb/' \
  -e "/usr/bin/ssh -o BatchMode=yes" \
  "$SRC" "$HOST:$STAGING/"

echo
echo "==> staged on $HOST at $STAGING"
/usr/bin/ssh -o BatchMode=yes "$HOST" "du -sh $STAGING"
cat <<EOF

Step two, when you are ready. This is the one that needs root, and it is done
through docker because you are in the docker group and sudo is not open:

  ssh $HOST
  docker run --rm -v /home/frank:/h alpine \\
    sh -c 'cp -rn /h/esde-staging/. /h/romm/resources/esde-media/ && echo done'

  # then, once the app shows the art, reclaim the space:
  rm -rf $STAGING

Both paths are written as the container sees them. /home/frank is mounted at /h,
so a host path on either side of that cp fails with "No such file or directory" —
which reads like the destination is missing when it is only spelled wrong.

-n keeps anything already on the server: the copy only adds. Both paths are on
the same disk, so it is fast and needs no second 90 GB.
EOF
