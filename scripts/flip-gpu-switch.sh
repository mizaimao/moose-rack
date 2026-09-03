#!/usr/bin/env bash
#
# Install two entries under Ports on the Flip for switching the GPU driver.
#
# The device ships `libmali-bifrost-g52-g13p0-gbm` — no Wayland, old GBM — and
# that is why no compositor starts. `g24p0-wayland-gbm` is the same vendor
# driver built with both. Emulators measured identical on either: same frame
# rate, byte-identical rendered frames. See
# docs/flip-wayland-and-the-gpu-blob.md.
#
#   ./scripts/flip-gpu-switch.sh install    put the two entries under Ports
#   ./scripts/flip-gpu-switch.sh remove     take them off, restore stock
#
# `/` on this device is an overlay whose writable layer is a **256 MB tmpfs** —
# so anything written to /usr/lib is gone at the next boot. A switch that only
# copies the file therefore appears to work and quietly reverts. The choice is
# recorded in /userdata (which is real storage) and re-applied at every boot by
# a hook in /boot, which is the earliest thing that runs.
#
# The same fact is the safety net: a driver that will not load is undone by
# pulling the power, because the swap never survived on its own.
set -euo pipefail

FLIP="${FLIP:-10.10.10.187}"
FLIP_PASSWORD="${FLIP_PASSWORD:-linux}"
GPU=/userdata/system/gpu
PORTS=/userdata/roms/ports
BLOB=libmali-bifrost-g52-g24p0-wayland-gbm.so
URL="https://raw.githubusercontent.com/ROCKNIX/libmali/master/lib/aarch64-linux-gnu/$BLOB"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="$HERE/.toolchain/$BLOB"

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }

ssh_do() {
  expect -c "
    set timeout 900
    log_user 0
    spawn ssh -tt -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null root@$FLIP {$1}
    expect \"assword:\" { send \"$FLIP_PASSWORD\r\" }
    log_user 1
    expect eof
  "
}
send() {
  expect -c "
    set timeout 1800
    log_user 0
    spawn scp -O -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null $1 root@$FLIP:$2
    expect \"assword:\" { send \"$FLIP_PASSWORD\r\" }
    expect eof" >/dev/null
}

# The script that does the work, written out here so it lives in the repo
# rather than only on a device.
# The script that does the work, written out here so it lives in the repo
# rather than only on a device.
switcher() {
cat <<'SWITCH'
#!/bin/sh
# Choose the Mali userspace driver. Run from Ports.
#
# This records the choice and applies it now; the boot hook re-applies it every
# start, because the writable layer of / is a tmpfs and forgets.
GPU=/userdata/system/gpu
LIVE=/usr/lib/libmali.so.1
say() { echo "$*"; echo "$*" > /dev/tty0 2>/dev/null; }

case "$1" in
  wayland) WANT=$GPU/libmali-g24p0-wayland.so ; NAME="g24p0 (Wayland)" ;;
  stock)   WANT=$GPU/libmali-g13p0-stock.so   ; NAME="g13p0 (stock)"   ;;
  *) echo "usage: $0 wayland|stock" ; exit 1 ;;
esac

[ -s "$WANT" ] || { say "GPU: $NAME is not on this device — nothing changed."; sleep 4; exit 1; }
# Never proceed without a way back.
[ -s "$GPU/libmali-g13p0-stock.so" ] || { say "GPU: no backup of the stock driver — refusing."; sleep 4; exit 1; }

echo "$1" > "$GPU/selected"
say "GPU: set to $NAME."
if cmp -s "$WANT" "$LIVE"; then
  say "Already running it. Nothing else to do."
  sleep 3; exit 0
fi
cp "$WANT" "$LIVE.new" && mv "$LIVE.new" "$LIVE" \
  && say "Applied. Reboot so everything picks it up." \
  || say "Could not swap the live file; it will be applied at the next boot."
sleep 5
SWITCH
}

# The boot hook. /boot is vfat and read-only at runtime, so it is remounted to
# write this once.
boothook() {
cat <<'HOOK'
#!/bin/bash
# Re-apply the chosen Mali driver at every boot.
#
# The writable layer of / is a tmpfs, so /usr/lib is the stock image again on
# every start. Whatever was chosen lives in /userdata, which is real storage.
case "$1" in
  start)
    GPU=/userdata/system/gpu
    [ -s "$GPU/selected" ] || exit 0
    WANT="$GPU/libmali-$(cat $GPU/selected 2>/dev/null | tr -d '\r\n')"
    case "$(cat $GPU/selected 2>/dev/null | tr -d '\r\n')" in
      wayland) WANT=$GPU/libmali-g24p0-wayland.so ;;
      stock)   WANT=$GPU/libmali-g13p0-stock.so ;;
      *) exit 0 ;;
    esac
    [ -s "$WANT" ] || exit 0
    cp "$WANT" /usr/lib/libmali.so.1.new 2>/dev/null && \
      mv /usr/lib/libmali.so.1.new /usr/lib/libmali.so.1 2>/dev/null
    ;;
esac
exit 0
HOOK
}

case "${1:-}" in
  install)
    if [ ! -s "$CACHE" ]; then
      say "fetching the Wayland-capable driver (56 MB)"
      mkdir -p "$(dirname "$CACHE")"
      curl -fL --progress-bar "$URL" -o "$CACHE"
    fi
    say "making $GPU and backing up the driver the device is running"
    ssh_do "mkdir -p $GPU && [ -s $GPU/libmali-g13p0-stock.so ] || cp /usr/lib/libmali.so.1 $GPU/libmali-g13p0-stock.so; ls -la $GPU" >/dev/null
    say "sending the Wayland driver"
    send "$CACHE" "$GPU/libmali-g24p0-wayland.so"

    say "installing the boot hook so the choice survives a reboot"
    boothook > /tmp/boot-custom.sh
    ssh_do "mount -o remount,rw /boot" >/dev/null 2>&1 || true
    send /tmp/boot-custom.sh "/boot/boot-custom.sh"
    rm -f /tmp/boot-custom.sh
    ssh_do "chmod +x /boot/boot-custom.sh 2>/dev/null; ls -la /boot/boot-custom.sh; mount -o remount,ro /boot 2>/dev/null" >/dev/null

    say "installing the switcher and its two Ports entries"
    switcher > /tmp/gpu-switch.sh
    send /tmp/gpu-switch.sh "$GPU/gpu-switch.sh"
    rm -f /tmp/gpu-switch.sh
    ssh_do "chmod +x $GPU/gpu-switch.sh; \
      printf '#!/bin/bash\n$GPU/gpu-switch.sh wayland\n' > '$PORTS/GPU driver - Wayland.sh'; \
      printf '#!/bin/bash\n$GPU/gpu-switch.sh stock\n' > '$PORTS/GPU driver - stock.sh'; \
      chmod +x '$PORTS/GPU driver - Wayland.sh' '$PORTS/GPU driver - stock.sh'; \
      ls -la '$PORTS' | grep GPU" 
    say "done — two entries under Ports. They take effect on reboot."
    ;;
  remove)
    say "restoring the stock driver and removing the entries"
    ssh_do "[ -s $GPU/libmali-g13p0-stock.so ] && cp $GPU/libmali-g13p0-stock.so /usr/lib/libmali.so.1 && echo 'stock driver restored'; \
      rm -f '$PORTS/GPU driver - Wayland.sh' '$PORTS/GPU driver - stock.sh'; rm -rf $GPU; \
      mount -o remount,rw /boot 2>/dev/null; rm -f /boot/boot-custom.sh; mount -o remount,ro /boot 2>/dev/null; echo removed"
    ;;
  status)
    ssh_do "if cmp -s $GPU/libmali-g24p0-wayland.so /usr/lib/libmali.so.1 2>/dev/null; then echo 'running now: g24p0 (Wayland)'; else echo 'running now: g13p0 (stock)'; fi; echo \"chosen for next boot: \$(cat $GPU/selected 2>/dev/null || echo '(none — stock)')\"; echo \"boot hook: \$([ -f /boot/boot-custom.sh ] && echo installed || echo missing)\""
    ;;
  *) sed -n '3,20p' "$0" | sed 's/^# \{0,1\}//'; exit 1 ;;
esac
