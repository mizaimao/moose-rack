#!/usr/bin/env bash
#
# Build moose-sdl for the Miyoo Flip running KNULLI, and put it there.
#
# Everything this installs lives in one folder inside the project:
# `.toolchain/`. Not /usr/local, not /opt/homebrew, not ~/.cargo/bin, and not
# ~/.rustup — it carries its own rust toolchain rather than adding a target to
# yours. `knulli.sh clean` is `rm -rf` on one directory and there is nothing
# else to find.
#
# That costs about a gigabyte more than sharing the toolchain already on the
# machine. It is the right trade for a thing installed to talk to one handheld:
# a large folder you can see and delete beats sixty megabytes in a path nobody
# will remember six months from now. `SHARED=1` takes the other side of it.
#
#   ./scripts/knulli.sh toolchain   fetch zig + cargo-zigbuild into .toolchain/
#   ./scripts/knulli.sh sysroot     copy the device's SDL2 libraries here
#   ./scripts/knulli.sh build       cross-compile for aarch64 KNULLI
#   ./scripts/knulli.sh install     send the build to the device
#   ./scripts/knulli.sh clean       remove every trace of the above
#   ./scripts/knulli.sh all         toolchain, sysroot, build, install
#
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KIT="$HERE/.toolchain"
ZIG_VERSION="0.15.1"
# Checked against the tarball after download. Not paranoia about ziglang.org —
# a truncated download unpacks into a compiler that fails in a way that reads
# like a bug in this project.
ZIG_SHA256="c4bd624d901c1268f2deb9d8eb2d86a2f8b97bafa3f118025344242da2c54d7b"
# The device: password auth, no keys. Overridable, because an address on a
# home network is not a constant — and this one has already moved once, from
# .187 to .186 across a reboot. The mDNS name is what the device answers to
# whatever DHCP decides, so that is the default now.
FLIP="${FLIP:-knulli.local}"
FLIP_PASSWORD="${FLIP_PASSWORD:-linux}"
# Where it lands on the device. Under /userdata because that is the persistent
# partition and the only one that is not read-only.
REMOTE="/userdata/system/moose-rack"
# The addon keeps its own folder: it outlives the front end, and its profile
# is the thing you carry to a freshly flashed device.
MOOSE="/userdata/system/moose-patch"

# The target triple, with the glibc the device actually has.
#
# KNULLI's is 2.40 (Buildroot). Asking zig for an older one is safe — a binary
# built against 2.36 runs on 2.40 — and asking for a newer one is a binary that
# will not start, with a message about a version node that names no file.
TARGET="aarch64-unknown-linux-gnu"
GLIBC="2.36"

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }

# zig names its tarballs architecture first — `zig-aarch64-macos-0.15.1`. It
# used to be the other way round, and the old spelling 404s.
case "$(uname -m)" in
  arm64|aarch64) ZIG_HOST="aarch64-macos" ;;
  *)             ZIG_HOST="x86_64-macos" ;;
esac
ZIG_NAME="zig-$ZIG_HOST-$ZIG_VERSION"
ZIG_DIR="$KIT/$ZIG_NAME"

cmd_toolchain() {
  mkdir -p "$KIT"
  if [ ! -x "$ZIG_DIR/zig" ]; then
    say "fetching zig $ZIG_VERSION ($ZIG_HOST)"
    local tarball="$KIT/zig.tar.xz"
    curl -fL --progress-bar \
      "https://ziglang.org/download/$ZIG_VERSION/$ZIG_NAME.tar.xz" \
      -o "$tarball"
    local got
    got="$(shasum -a 256 "$tarball" | cut -d" " -f1)"
    if [ "$got" != "$ZIG_SHA256" ]; then
      echo "zig checksum mismatch: got $got, expected $ZIG_SHA256" >&2
      rm -f "$tarball"
      exit 1
    fi
    tar -xJf "$tarball" -C "$KIT"
    rm -f "$tarball"
  fi
  [ -x "$ZIG_DIR/zig" ] || { echo "zig did not unpack to $ZIG_DIR" >&2; exit 1; }

  if [ ! -x "$KIT/cargo/bin/cargo-zigbuild" ]; then
    say "building cargo-zigbuild into .toolchain (not ~/.cargo/bin)"
    cargo install cargo-zigbuild --root "$KIT/cargo" --locked
  fi

  # A rust toolchain of our own, inside the project.
  #
  # `rustup target add` writes into ~/.rustup, and a folder somebody will not
  # remember is a folder that stays on the machine forever. `RUSTUP_HOME` moves
  # the whole thing here instead, so removing this build means removing one
  # directory and nothing else.
  #
  # It costs a second copy of rustc and cargo — about a gigabyte, against sixty
  # megabytes for adding a target to the toolchain that is already installed.
  # That is the trade: disk inside a folder you can delete, rather than a small
  # thing you cannot find. `SHARED=1` takes the other side of it.
  if [ "${SHARED:-0}" = "1" ]; then
    say "adding the rust standard library for $TARGET to ~/.rustup (SHARED=1)"
    say "  remove later with: rustup target remove $TARGET"
    rustup target add "$TARGET"
  else
    say "installing a private rust toolchain in .toolchain/rustup"
    # The same version the project pins, or a cross build and a native build
    # are two different compilers and only one of them is the one CI runs.
    local channel
    channel="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$HERE/rust-toolchain.toml")"
    RUSTUP_HOME="$KIT/rustup" rustup toolchain install "$channel" \
      --profile minimal --target "$TARGET" --no-self-update
  fi

  say "toolchain ready in .toolchain/ — 'clean' removes it"
}

# The device's own SDL2, to link against.
#
# Only the SDL libraries, not the whole 1.6 GB of /usr/lib: the linker needs
# these two by name and everything underneath them is resolved on the device at
# run time, which is what --allow-shlib-undefined below says.
cmd_sysroot() {
  local lib="$KIT/sysroot/usr/lib"
  mkdir -p "$lib/pkgconfig"
  say "copying SDL2 from $FLIP"
  scp_from "/usr/lib/libSDL2-2.0.so.0.3200.8" "$lib/libSDL2.so"
  scp_from "/usr/lib/libSDL2_image-2.0.so.0.800.5" "$lib/libSDL2_image.so"

  # Two files describing what was just copied.
  #
  # `sdl2-sys` is built with `use-pkgconfig`, and that is deliberate: on this
  # Mac SDL lives in /opt/homebrew/lib, which the linker does not search, and
  # the failure without it is `library 'SDL2' not found` with nothing saying the
  # library is installed one directory away. So rather than special-case the
  # dependency for one target, pkg-config is given a sysroot it can answer
  # from — which is what a cross build is supposed to look like anyway.
  say "writing pkg-config descriptions for them"
  cat > "$lib/pkgconfig/sdl2.pc" <<PC
prefix=/usr
libdir=\${prefix}/lib
includedir=\${prefix}/include

Name: sdl2
Description: SDL2, as copied off the device
Version: 2.32.8
Libs: -L\${libdir} -lSDL2
Cflags: -I\${includedir}/SDL2 -D_REENTRANT
PC
  cat > "$lib/pkgconfig/SDL2_image.pc" <<PC
prefix=/usr
libdir=\${prefix}/lib
includedir=\${prefix}/include

Name: SDL2_image
Description: SDL2_image, as copied off the device
Version: 2.8.5
Requires: sdl2
Libs: -L\${libdir} -lSDL2_image
Cflags: -I\${includedir}/SDL2
PC
  ls -la "$lib"
}

cmd_build() {
  [ -x "$ZIG_DIR/zig" ] || { echo "run 'knulli.sh toolchain' first" >&2; exit 1; }
  [ -f "$KIT/sysroot/usr/lib/libSDL2.so" ] || { echo "run 'knulli.sh sysroot' first" >&2; exit 1; }

  say "building moose-sdl and moose-patch for $TARGET (glibc $GLIBC)"
  # `--allow-shlib-undefined`: libSDL2.so names two dozen libraries of its own
  # and every one of them is on the device. Copying all of them here to satisfy
  # a link that does not need them is 1.6 GB for nothing.
  # The private toolchain, unless the shared one was asked for. `rust-toolchain.toml`
  # names a channel, and a RUSTUP_HOME that does not have it would try to fetch
  # it mid-build — so the toolchain step installs exactly that one.
  local home_args=()
  if [ "${SHARED:-0}" != "1" ] && [ -d "$KIT/rustup" ]; then
    home_args=(env "RUSTUP_HOME=$KIT/rustup")
  fi

  # `sdl2-sys` asks pkg-config where SDL2 is, and pkg-config refuses to answer
  # for a target that is not this machine. There is nothing to ask: the library
  # is the one copied off the device and sitting in .toolchain/sysroot, so the
  # build script is told to skip the question and just link the name.
  # pkg-config, pointed at the sysroot rather than at this machine. Without
  # ALLOW_CROSS it refuses to answer at all for a target that is not the host,
  # and LIBDIR rather than PATH so it cannot fall back to the Mac's own .pc
  # files and hand the linker /opt/homebrew.
  PATH="$ZIG_DIR:$KIT/cargo/bin:$PATH" \
  PKG_CONFIG_ALLOW_CROSS=1 \
  PKG_CONFIG_SYSROOT_DIR="$KIT/sysroot" \
  PKG_CONFIG_LIBDIR="$KIT/sysroot/usr/lib/pkgconfig" \
  RUSTFLAGS="-C link-arg=-Wl,--allow-shlib-undefined" \
    "${home_args[@]}" cargo zigbuild \
      --release \
      -p moose-sdl -p moose-patch \
      --features knulli \
      --target "$TARGET.$GLIBC"

  for out in "$HERE/target/$TARGET/release/moose-sdl" \
             "$HERE/target/$TARGET/release/moose-patch"; do
    [ -f "$out" ] || continue
    file "$out" || true
    say "built $(du -h "$out" | cut -f1) at $out"
  done
}

cmd_install() {
  local out="$HERE/target/$TARGET/release/moose-sdl"
  [ -f "$out" ] || { echo "nothing built; run 'knulli.sh build'" >&2; exit 1; }

  # Stop it first. A running executable cannot be overwritten.
  say "stopping anything already running on $FLIP"
  ssh_do "pkill -x moose-sdl; sleep 1" >/dev/null 2>&1 || true

  say "sending to $FLIP:$REMOTE"
  ssh_do "mkdir -p $REMOTE/data"
  scp_to "$out" "$REMOTE/moose-sdl"
  # The tables the app reads at run time, beside the binary — it looks for
  # `data/esde-core-map.json` relative to where it is started from.
  for f in esde-core-map.json icon-set-art.toml arcade-names.json; do
    [ -f "$HERE/data/$f" ] && scp_to "$HERE/data/$f" "$REMOTE/data/$f"
  done
  scp_to "$HERE/scripts/moose-rack-launch.sh" "$REMOTE/moose-rack-launch.sh"
  ssh_do "chmod +x $REMOTE/moose-sdl $REMOTE/moose-rack-launch.sh"

  # The addon. Everything it needs to patch a device is compiled into it, so
  # this is one binary and one launcher — no assets to keep beside it.
  local addon="$HERE/target/$TARGET/release/moose-patch"
  if [ -f "$addon" ]; then
    say "sending moose-patch to $FLIP:$MOOSE"
    ssh_do "mkdir -p $MOOSE"
    scp_to "$addon" "$MOOSE/moose-patch"
    scp_to "$HERE/scripts/moose-launch.sh" "$MOOSE/moose-launch.sh"
    ssh_do "chmod +x $MOOSE/moose-patch $MOOSE/moose-launch.sh"
    ssh_do "cp $MOOSE/moose-launch.sh /userdata/roms/ports/moose-patch.sh && chmod +x /userdata/roms/ports/moose-patch.sh"
  fi

  # A Ports entry, so it starts the way everything else on the device does.
  ssh_do "cp $REMOTE/moose-rack-launch.sh '/userdata/roms/ports/Moose Rack.sh' && chmod +x '/userdata/roms/ports/Moose Rack.sh'"

  # L2+R2 is not bound here. moose-patch's own "hotkey-app" patch owns that
  # rule, so binding it from the installer as well would put two copies in the
  # file and leave the app unable to say honestly whether it is on. The Ports
  # entry above is how you reach it on a device that has never run it.

  # Leave the device showing something.
  #
  # This step stops Moose Rack so the binary can be overwritten, and a testing session
  # may have stopped EmulationStation before that. Between the two the handheld
  # is a black screen, and it stays that way until somebody notices. Nothing
  # here should be able to leave it dark.
  # Leave a front end on screen, and *say* whether there is one.
  #
  # The first version of this ran the check inside a compound command with its
  # output thrown away, so it could fail silently — and it did: the step
  # announced it was making sure, and the device stayed black. A check whose
  # result nobody looks at is not a check.
  say "making sure the device has a front end on screen"
  ssh_do "pidof emulationstation >/dev/null && echo ES-UP || echo ES-DOWN" > "$KIT/es.state" 2>&1 || true
  if grep -q ES-DOWN "$KIT/es.state" 2>/dev/null; then
    say "  nothing on screen — starting EmulationStation"
    # The wrapper directly, not the init script.
    #
    # `/etc/init.d/S31emulationstation start` consults a setting and hands off,
    # and there are states where it returns having started nothing. The wrapper
    # is what actually runs EmulationStation — it recreates its own restart flag
    # and loops — and started detached it survives this ssh session ending.
    ssh_do "cat /dev/zero > /dev/fb0 2>/dev/null; setsid nohup /usr/bin/emulationstation-standalone >/tmp/es.log 2>&1 </dev/null & sleep 18; pidof emulationstation >/dev/null && echo ES-UP || echo ES-DOWN" > "$KIT/es.state" 2>&1 || true
    if grep -q ES-DOWN "$KIT/es.state" 2>/dev/null; then
      echo "  WARNING: the device has no front end on screen" >&2
    else
      say "  EmulationStation is back"
    fi
  else
    say "  EmulationStation is running"
  fi
  rm -f "$KIT/es.state"
  say "installed — it is under Ports as 'Moose Rack'"
}

cmd_clean() {
  say "removing .toolchain/ ($(du -sh "$KIT" 2>/dev/null | cut -f1 || echo nothing))"
  rm -rf "$KIT"
  say "removing the cross build output"
  rm -rf "$HERE/target/$TARGET"

  # Only if the shared home was used. Removing a target somebody added for their
  # own reasons would be worse than leaving one behind.
  if rustup target list --installed 2>/dev/null | grep -qx "$TARGET"; then
    say "the shared toolchain also has $TARGET installed"
    say "  that is from SHARED=1, or from something else you did"
    say "  remove it with: rustup target remove $TARGET"
  fi

  cat <<'NOTE'

Gone. Everything this script installed lived in .toolchain/ inside the project,
including its own copy of rustc — nothing was written to ~/.rustup, /usr/local,
/opt/homebrew or ~/.cargo/bin. What is left on this machine that was not here
before is downloaded crates in ~/.cargo/registry, which every build in this
project already shares.

To check for yourself:  ls ~/.rustup/toolchains
NOTE
}

# --- talking to the device -------------------------------------------------
#
# Password auth through `expect`. Not keys: KNULLI's sshd refuses them because
# StrictModes sees a 0777 home, and an earlier attempt lost a day to that.

ssh_do() {
  expect -c "
    set timeout 120
    log_user 0
    spawn ssh -tt -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null root@$FLIP {$1}
    expect \"assword:\" { send \"$FLIP_PASSWORD\r\" }
    log_user 1
    expect eof
  "
}

# Braces only quote at the start of a word in Tcl.
#
# `root@host:{$1}` is not a quoted path — it is `root@host:` followed by a
# literal open brace — so scp went looking for a file with braces in its name
# and failed, while the `echo` below said it had worked. Hence no braces here,
# and a check afterwards rather than a cheerful message.
scp_to() {
  expect -c "
    set timeout 600
    log_user 0
    spawn scp -O -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null $1 root@$FLIP:$2
    expect \"assword:\" { send \"$FLIP_PASSWORD\r\" }
    expect eof
  " >/dev/null
  # Checked, not announced.
  #
  # scp reports "text file busy" and gives up when the target is a running
  # executable — which is exactly the case here, because testing means the app
  # is often up when a new build is sent. The old version stayed on the device,
  # the script cheerfully said "sent", and an hour went into wondering why a
  # change that was plainly in the source was not in the running program.
  # Ask the device for the hash and look for ours in the answer, rather than
  # trying to extract exactly one field through two layers of quoting — ssh
  # brings a login banner and carriage returns with it, and a comparison that
  # has to survive all of that is a comparison that fails for its own reasons.
  local want
  want="$(shasum -a 256 "$1" | cut -d" " -f1)"
  if ! ssh_do "sha256sum $2" 2>/dev/null | grep -q "$want"; then
    echo "  FAILED to send $(basename "$1") — the copy on the device does not match" >&2
    echo "  (is it running? scp will not overwrite a busy executable)" >&2
    exit 1
  fi
  echo "  sent $(basename "$1") ($(du -h "$1" | cut -f1))"
}

scp_from() {
  expect -c "
    set timeout 600
    log_user 0
    spawn scp -O -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null root@$FLIP:$1 $2
    expect \"assword:\" { send \"$FLIP_PASSWORD\r\" }
    expect eof
  " >/dev/null
  if [ ! -s "$2" ]; then
    echo "failed to copy $1 off the device" >&2
    exit 1
  fi
  echo "  got $(basename "$2") ($(du -h "$2" | cut -f1))"
}

case "${1:-}" in
  toolchain) cmd_toolchain ;;
  sysroot)   cmd_sysroot ;;
  build)     cmd_build ;;
  install)   cmd_install ;;
  clean)     cmd_clean ;;
  all)       cmd_toolchain; cmd_sysroot; cmd_build; cmd_install ;;
  *) sed -n '3,20p' "$0" | sed 's/^# \{0,1\}//' ; exit 1 ;;
esac
