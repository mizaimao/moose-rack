#!/usr/bin/env bash
#
# Put the newer Mali blob on the Flip *temporarily*, so it can be tried.
#
# The device ships `libmali-bifrost-g52-g13p0-gbm` — the GBM-only, no-Wayland
# build — and that is why no compositor starts. This stages
# `g24p0-wayland-gbm` under /tmp and hands back the one line to run things
# with. Nothing is installed: `/usr/lib` is untouched, and a reboot forgets it.
#
#   ./scripts/flip-mali-shim.sh stage     fetch and put it on the device
#   ./scripts/flip-mali-shim.sh remove    take it off again
#   ./scripts/flip-mali-shim.sh status    say whether it is staged
#
# See docs/flip-wayland-and-the-gpu-blob.md for what it is for.
set -euo pipefail

FLIP="${FLIP:-10.10.10.187}"
FLIP_PASSWORD="${FLIP_PASSWORD:-linux}"
REMOTE=/tmp/mali24
BLOB=libmali-bifrost-g52-g24p0-wayland-gbm.so
URL="https://raw.githubusercontent.com/ROCKNIX/libmali/master/lib/aarch64-linux-gnu/$BLOB"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="$HERE/.toolchain/$BLOB"

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }

ssh_do() {
  expect -c "
    set timeout 600
    log_user 0
    spawn ssh -tt -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null root@$FLIP {$1}
    expect \"assword:\" { send \"$FLIP_PASSWORD\r\" }
    log_user 1
    expect eof
  "
}

case "${1:-status}" in
  stage)
    if [ ! -s "$CACHE" ]; then
      say "fetching the blob (56 MB)"
      mkdir -p "$(dirname "$CACHE")"
      curl -fL --progress-bar "$URL" -o "$CACHE"
    fi
    say "sending it to $FLIP:$REMOTE"
    ssh_do "mkdir -p $REMOTE" >/dev/null
    expect -c "
      set timeout 900
      log_user 0
      spawn scp -O -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null $CACHE root@$FLIP:$REMOTE/libmali.so.1
      expect \"assword:\" { send \"$FLIP_PASSWORD\r\" }
      expect eof" >/dev/null
    # The blob is one mega-library reached under several names. Every name the
    # system points at the stock one has to point at this one too, or half the
    # stack is old and half is new.
    ssh_do "cd $REMOTE && for n in libEGL.so.1 libGLESv2.so.2 libGLESv1_CM.so.1 libgbm.so.1 libwayland-egl.so.1 libMali.so; do ln -sf libmali.so.1 \$n; done && ls $REMOTE | wc -l" >/dev/null
    say "staged. Run anything against it with:"
    echo
    echo "    LD_LIBRARY_PATH=$REMOTE <command>"
    echo
    echo "  a compositor:  LD_LIBRARY_PATH=$REMOTE weston --backend=drm-backend.so --idle-time=0"
    echo "  an emulator:   LD_LIBRARY_PATH=$REMOTE retroarch -L /usr/lib/libretro/flycast_libretro.so ROM"
    echo
    echo "  (stop EmulationStation first — see docs/flip-wayland-and-the-gpu-blob.md)"
    ;;
  remove)
    say "removing $REMOTE"
    ssh_do "rm -rf $REMOTE; ls -la /usr/lib/libgbm.so.1" 
    say "gone; /usr/lib was never written to"
    ;;
  status)
    ssh_do "test -d $REMOTE && echo 'staged at $REMOTE' || echo 'not staged'"
    ;;
  *) sed -n '3,16p' "$0" | sed 's/^# \{0,1\}//'; exit 1 ;;
esac
