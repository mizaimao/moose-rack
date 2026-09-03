#!/usr/bin/env bash
#
# Build moose-gui for Android and put it on the device.
#
# Everything this installs lives in one folder inside the project:
# `.toolchain/android/`. Nothing is written anywhere else on the machine —
# not ~/.gradle, not ~/.android, not /usr/local, not ~/.cargo, not ~/.rustup,
# and no Android Studio. `android.sh clean` is `rm -rf` on one directory.
#
# That is stricter than scripts/knulli.sh, which leaves crate downloads in the
# shared ~/.cargo/registry. Here CARGO_HOME points inside the kit too, so the
# first build re-fetches and recompiles every dependency from cold. `SHARED=1`
# takes the other side of that trade — the same flag knulli.sh uses.
#
#   ./scripts/android.sh toolchain  fetch JDK, SDK, NDK, cargo-ndk into .toolchain/
#   ./scripts/android.sh init       generate src-tauri/gen/android/
#   ./scripts/android.sh doctor     say where every tool is and prove nothing leaked
#   ./scripts/android.sh devices    list attached devices
#   ./scripts/android.sh dev        build, install and run on the device, with reload
#   ./scripts/android.sh build      build a release APK (unsigned)
#   ./scripts/android.sh install    build a debug APK and put it on the device
#   ./scripts/android.sh logs       follow the app's log on the device
#   ./scripts/android.sh shell      a shell on the device
#   ./scripts/android.sh env        print the environment, to eval in your own shell
#   ./scripts/android.sh clean      remove every trace of the above
#
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KIT="$HERE/.toolchain/android"

# The two things that are downloaded rather than resolved.
#
# Pinned and hash-checked for the reason knulli.sh pins zig: a truncated
# download unpacks into a compiler or an SDK that fails in a way that reads
# like a bug in this project, and the hours go to the wrong place.
JDK_URL="https://github.com/adoptium/temurin17-binaries/releases/download/jdk-17.0.20.1%2B1/OpenJDK17U-jdk_aarch64_mac_hotspot_17.0.20.1_1.tar.gz"
JDK_SHA256="196d13ba5f10414bef7f6a05a9b3f00edacb18ebacef2b99485db9e2ee18f0e8"
# JDK 17 and not something newer: that is what the Android Gradle Plugin the
# Tauri template generates is built against.

CLI_URL="https://dl.google.com/android/repository/commandlinetools-mac_arm64-15859902_latest.zip"
CLI_SHA256="835b62a26162b229b441d1f6d4680383815a270809eb33522c0d480fa5002c4e"

# The Thor is arm64. One target, not the usual four — three quarters of an
# Android build's time goes on architectures no device here runs.
#
# Two spellings of the same thing, and they are not interchangeable. rustup and
# the target/ directory want the triple; `tauri android build -t` takes an ABI
# name and rejects the triple outright with a list of the four it accepts.
TARGET="aarch64-linux-android"
ABI="aarch64"

# The NDK is resolved rather than pinned.
#
# docs/android-port.md wants r28 or later, for Google's 16 KB page alignment.
# Which exact build number is current changes every few weeks and a wrong pin
# fails with sdkmanager listing a hundred packages, so the newest available is
# picked at install time and the choice is printed and checked.
NDK_MIN_MAJOR=28

say()  { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m  warning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# --- the containment ------------------------------------------------------
#
# One function, called by every subcommand, that decides where everything
# writes. Each variable below is the officially supported way to move one
# tool's storage; the list is from developer.android.com/tools/variables.
#
# HOME is the backstop, and it is the important one. The named variables cover
# the tools we know about. A redirected HOME covers the ones we do not: a
# Gradle plugin, a Java preferences write, some helper that has its own idea of
# a dotfile. With it set, the worst case is a stray directory inside the kit
# rather than a stray directory in Frank's home.
android_env() {
  [ -d "$KIT/jdk" ] || die "no toolchain — run './scripts/android.sh toolchain' first"

  export JAVA_HOME="$KIT/jdk/Contents/Home"
  export ANDROID_HOME="$KIT/sdk"
  export ANDROID_SDK_ROOT="$KIT/sdk"

  # ~/.android — adb's device key, the emulator's config, analytics opt-outs.
  #
  # ANDROID_USER_HOME only, and deliberately not ANDROID_SDK_HOME as well. That
  # older variable names the *parent* of `.android` rather than `.android`
  # itself, so setting both to cover old tools pointed them at two different
  # folders — and the Android Gradle Plugin refuses to start when it finds
  # disagreeing answers rather than picking one:
  #
  #     Several environment variables and/or system properties contain
  #     different paths to the Android Preferences folder.
  #
  # Nothing is lost by dropping it. A tool too old to read ANDROID_USER_HOME
  # falls back to $HOME/.android, and HOME is redirected below — so it still
  # lands inside the kit.
  export ANDROID_USER_HOME="$KIT/android-user"
  export ANDROID_EMULATOR_HOME="$KIT/android-user/emulator"
  export ANDROID_AVD_HOME="$KIT/android-user/avd"

  # ~/.gradle — wrapper distributions, the build cache, daemon logs. This is
  # the largest single thing after the NDK, comfortably a gigabyte.
  export GRADLE_USER_HOME="$KIT/gradle"

  export RUSTUP_HOME="$KIT/rustup"
  if [ "${SHARED:-0}" = "1" ]; then
    warn "SHARED=1 — crates will be cached in ~/.cargo/registry, outside the kit"
  else
    export CARGO_HOME="$KIT/cargo"
  fi

  export TMPDIR="$KIT/tmp"
  export HOME="$KIT/home"

  # HOME is not enough, because the JVM does not read it.
  #
  # On macOS Java takes `user.home` from the password database, not from the
  # environment, so every variable above is invisible to it. Two things escaped
  # through that hole and were found in Frank's home directory after a build:
  #
  #     ~/.android/analytics.settings                    (the Gradle plugin)
  #     ~/Library/Application Support/kotlin/daemon/     (the Kotlin compiler)
  #
  # JAVA_TOOL_OPTIONS rather than GRADLE_OPTS: the build is not one JVM. Gradle
  # launches a daemon, which launches a Kotlin compile daemon, and only this is
  # picked up by all of them. It costs a "Picked up JAVA_TOOL_OPTIONS" line on
  # stderr from each, which is the whole price.
  export JAVA_TOOL_OPTIONS="-Duser.home=$KIT/home"

  local ndk
  ndk="$(ls -d "$KIT/sdk/ndk/"* 2>/dev/null | sort -V | tail -1 || true)"
  if [ -n "$ndk" ]; then
    export NDK_HOME="$ndk"
    export ANDROID_NDK_ROOT="$ndk"
    export ANDROID_NDK_HOME="$ndk"
  fi

  mkdir -p "$TMPDIR" "$HOME" "$ANDROID_USER_HOME" "$GRADLE_USER_HOME"

  # The kit's own binaries first, then the real cargo/rustup proxies, which
  # read the *_HOME variables above and so stay contained.
  export PATH="$JAVA_HOME/bin:$KIT/sdk/platform-tools:$KIT/sdk/cmdline-tools/latest/bin:$KIT/cargo/bin:$PATH"
}

fetch() {
  local url="$1" want="$2" out="$3"
  say "fetching $(basename "$out")"
  curl -fL --progress-bar "$url" -o "$out"
  local got
  got="$(shasum -a 256 "$out" | cut -d" " -f1)"
  if [ "$got" != "$want" ]; then
    rm -f "$out"
    die "checksum mismatch for $(basename "$out"): got $got, expected $want"
  fi
}

# --- toolchain ------------------------------------------------------------

cmd_toolchain() {
  mkdir -p "$KIT/tmp" "$KIT/home"

  if [ ! -x "$KIT/jdk/Contents/Home/bin/java" ]; then
    local tar="$KIT/tmp/jdk.tar.gz"
    fetch "$JDK_URL" "$JDK_SHA256" "$tar"
    say "unpacking the JDK"
    mkdir -p "$KIT/jdk"
    # --strip-components: the tarball's top level is `jdk-17.0.20.1+1/`, and a
    # version number in the path is a path that breaks on the next JDK bump.
    tar -xzf "$tar" -C "$KIT/jdk" --strip-components=1
    rm -f "$tar"
  fi
  [ -x "$KIT/jdk/Contents/Home/bin/java" ] || die "the JDK did not unpack to $KIT/jdk"

  if [ ! -x "$KIT/sdk/cmdline-tools/latest/bin/sdkmanager" ]; then
    local zip="$KIT/tmp/cmdline-tools.zip"
    fetch "$CLI_URL" "$CLI_SHA256" "$zip"
    say "unpacking the command line tools"
    # sdkmanager refuses to run unless it sits at `cmdline-tools/latest/`, and
    # the zip unpacks to a bare `cmdline-tools/`. Renaming is the whole fix,
    # and without it the failure is a Java stack trace about a missing repo.
    mkdir -p "$KIT/sdk/cmdline-tools"
    rm -rf "$KIT/tmp/unzip"
    unzip -q "$zip" -d "$KIT/tmp/unzip"
    mv "$KIT/tmp/unzip/cmdline-tools" "$KIT/sdk/cmdline-tools/latest"
    rm -rf "$KIT/tmp/unzip" "$zip"
  fi

  android_env

  # Licences, into $ANDROID_HOME/licenses. Accepting these is unavoidable —
  # Gradle refuses to build without them — and they are written inside the kit.
  say "accepting SDK licences (written to $ANDROID_HOME/licenses)"
  yes 2>/dev/null | sdkmanager --licenses >/dev/null || true

  say "resolving the newest NDK"
  local ndk_pkg ndk_major
  ndk_pkg="$(sdkmanager --list 2>/dev/null \
    | awk -F'|' '$1 ~ /^ *ndk;/ { gsub(/ /, "", $1); print $1 }' \
    | sort -V | tail -1)"
  [ -n "$ndk_pkg" ] || die "sdkmanager listed no ndk packages"
  ndk_major="${ndk_pkg#ndk;}"; ndk_major="${ndk_major%%.*}"
  if [ "$ndk_major" -lt "$NDK_MIN_MAJOR" ]; then
    die "newest NDK is $ndk_pkg, but r$NDK_MIN_MAJOR or later is needed for 16 KB page alignment"
  fi
  say "  $ndk_pkg"

  say "installing platform-tools, build-tools, a platform and the NDK (a few GB)"
  sdkmanager --install "platform-tools" "platforms;android-35" "build-tools;35.0.0" "$ndk_pkg"

  # The Android standard library, in the kit's own rustup home. `rustup target
  # add` against the shared home would write to ~/.rustup, which is exactly
  # what this script exists to avoid.
  local channel
  channel="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$HERE/rust-toolchain.toml")"
  say "installing rust $channel with $TARGET into the kit"
  rustup toolchain install "$channel" --profile minimal --target "$TARGET" --no-self-update

  # cargo-ndk into the kit, not ~/.cargo/bin.
  if [ ! -x "$KIT/cargo/bin/cargo-ndk" ]; then
    say "building cargo-ndk into the kit"
    cargo install cargo-ndk --root "$KIT/cargo" --locked
  fi

  say "toolchain ready in .toolchain/android — 'clean' removes it"
  cmd_doctor
}

# --- the project ----------------------------------------------------------

tauri() {
  # The Tauri CLI from node_modules, which is already inside the project.
  # Not a global npm install, and not `cargo install tauri-cli`.
  #
  # Through `npm run`, and that is not a style choice.
  #
  # `android init` bakes this invocation into the generated Gradle project,
  # which re-runs it later from `src-tauri/` to build the .so. The CLI does not
  # record the path it was started from — it reconstructs a package-manager
  # command — so running the binary directly (either through the
  # `node_modules/.bin/tauri` symlink or as `node .../tauri.js`) leaves it with
  # nothing to name but the bare word, and the generated task becomes
  #
  #     node tauri android android-studio-script
  #
  # run from a directory with no `tauri` in it:
  #
  #     Error: Cannot find module '.../src-tauri/tauri'
  #
  # Running through npm gives it a package manager to name, so it bakes one
  # that works from anywhere. That is why package.json carries a `tauri` script
  # that looks redundant — the Gradle build is its only caller.
  [ -f "$HERE/node_modules/@tauri-apps/cli/tauri.js" ] || die "no tauri CLI — run 'npm install' first"
  ( cd "$HERE" && npm run --silent tauri -- "$@" )
}

cmd_init() {
  android_env
  if [ -d "$HERE/src-tauri/gen/android" ]; then
    say "src-tauri/gen/android already exists — nothing to do"
    return
  fi
  say "generating the Gradle project"
  # Output lands in src-tauri/gen/android, which .gitignore already covers.
  tauri android init
}

cmd_install() {
  android_env
  [ -d "$HERE/src-tauri/gen/android" ] || die "run './scripts/android.sh init' first"
  adb get-state >/dev/null 2>&1 || die "no device — './scripts/android.sh devices' says why"

  # A debug APK, because a release one is unsigned and Android will not install
  # it. Gradle signs debug builds with a throwaway key it generates itself,
  # into ANDROID_USER_HOME — so that key lands in the kit like everything else
  # and there is no keystore to manage.
  say "building a debug APK"
  tauri android build --debug --apk --target "$ABI"

  local apk
  apk="$(find "$HERE/src-tauri/gen/android/app/build/outputs/apk" -name '*debug*.apk' \
    | sort | tail -1)"
  [ -n "$apk" ] || die "the build produced no debug APK"

  say "installing $(basename "$apk") ($(du -h "$apk" | cut -f1))"
  # -r replaces an existing install and keeps its data.
  adb install -r "$apk"
  say "installed — start it from the launcher, or './scripts/android.sh logs'"
}

cmd_dev() {
  android_env
  [ -d "$HERE/src-tauri/gen/android" ] || die "run './scripts/android.sh init' first"
  say "building, installing and running on the device"
  say "  edits under ui/ reload on the device without a rebuild"
  say "  attach a debugger from desktop Chrome at chrome://inspect#devices"
  tauri android dev
}

cmd_build() {
  android_env
  [ -d "$HERE/src-tauri/gen/android" ] || die "run './scripts/android.sh init' first"
  say "building an APK for $ABI ($TARGET)"
  tauri android build --apk --target "$ABI"
  # The APK is unsigned unless a keystore is configured. Say so rather than
  # letting an install fail on the device with a signature error.
  find "$HERE/src-tauri/gen/android" -name '*.apk' -newermt '-10 minutes' 2>/dev/null \
    | while read -r apk; do echo "  $apk ($(du -h "$apk" | cut -f1))"; done
  say "unsigned unless a keystore is set up — 'dev' installs a debug build for you"
}

# --- the device -----------------------------------------------------------
#
# adb is a singleton: one server per machine, on port 5037. If one is already
# running that was started with the ordinary HOME — by Android Studio, scrcpy,
# anything — our client talks to *that* server and uses *its* key, which lives
# in ~/.android. Nothing of ours is written there, but the containment is only
# as good as which server answers. `adb kill-server` first if in doubt.

cmd_devices() {
  android_env
  adb devices -l
  say "no device? enable Developer options, then USB debugging, and accept the prompt"
  say "wireless: 'adb tcpip 5555' over USB, then 'adb connect <ip>:5555'"
}

cmd_logs() {
  android_env
  # The app's own output plus the webview's console, and nothing else — an
  # unfiltered logcat on a modern Android device is unreadable.
  say "following the log (ctrl-c to stop)"
  adb logcat -v color RustStdoutStderr:V moose_rack:V moose_gui:V chromium:V Tauri:V '*:S'
}

cmd_shell() { android_env; adb shell; }

cmd_env() {
  android_env
  for v in JAVA_HOME ANDROID_HOME ANDROID_SDK_ROOT ANDROID_USER_HOME \
           ANDROID_EMULATOR_HOME ANDROID_AVD_HOME GRADLE_USER_HOME RUSTUP_HOME \
           CARGO_HOME NDK_HOME ANDROID_NDK_ROOT TMPDIR HOME JAVA_TOOL_OPTIONS; do
    [ -n "${!v:-}" ] && printf 'export %s=%q\n' "$v" "${!v}"
  done
}

cmd_doctor() {
  android_env
  say "where everything is"
  for v in JAVA_HOME ANDROID_HOME ANDROID_USER_HOME GRADLE_USER_HOME RUSTUP_HOME \
           CARGO_HOME NDK_HOME TMPDIR HOME; do
    printf '  %-20s %s\n' "$v" "${!v:-(unset)}"
  done

  say "versions"
  printf '  %-20s %s\n' java "$(java -version 2>&1 | head -1)"
  printf '  %-20s %s\n' adb "$(adb --version 2>/dev/null | head -1 || echo missing)"
  printf '  %-20s %s\n' cargo-ndk "$(cargo ndk --version 2>/dev/null || echo missing)"

  # The check that matters: did anything land outside the kit?
  say "checking nothing leaked onto the machine"
  local leaked=0
  # The last two are where it actually got out before JAVA_TOOL_OPTIONS was
  # set — the JVM ignores HOME on macOS and reads the password database. Dates
  # are printed rather than judged: this script cannot know whether something
  # here predates it, and guessing would be worse than saying when.
  for d in "$REAL_HOME/.android" "$REAL_HOME/.gradle" "$REAL_HOME/Library/Android" \
           "$REAL_HOME/Library/Application Support/kotlin"; do
    if [ -e "$d" ]; then
      warn "$d exists, last written $(date -r "$d" '+%Y-%m-%d %H:%M' 2>/dev/null || echo unknown)"
      leaked=1
    fi
  done
  if [ "${SHARED:-0}" = "1" ]; then
    warn "SHARED=1: ~/.cargo/registry is in use, deliberately"
    leaked=1
  fi
  [ "$leaked" = "0" ] && say "  clean — everything is inside .toolchain/android"
  say "kit size: $(du -sh "$KIT" 2>/dev/null | cut -f1 || echo 0)"
}

cmd_clean() {
  say "removing .toolchain/android ($(du -sh "$KIT" 2>/dev/null | cut -f1 || echo nothing))"
  # Stop the daemons first. A Gradle daemon holds files open under
  # GRADLE_USER_HOME and lives for hours after a build; an adb server started
  # from here keeps running too. Removing the directory under them leaves two
  # processes pointing at nothing.
  if [ -d "$KIT/jdk" ]; then
    ( android_env; adb kill-server >/dev/null 2>&1 || true )
    pkill -f "GradleDaemon" >/dev/null 2>&1 || true
  fi
  rm -rf "$KIT"
  say "removing the Android build output"
  rm -rf "$HERE/target/$TARGET" "$HERE/src-tauri/gen/android"

  cat <<'NOTE'

Gone. Everything this script installed lived in .toolchain/android/ inside the
project — its own JDK, its own SDK and NDK, its own Gradle home, its own rustup
and cargo homes, and a redirected HOME to catch anything that had other ideas.

Nothing was written to ~/.gradle, ~/.android, ~/Library/Android, ~/.rustup,
~/.cargo, /usr/local or /opt/homebrew.

To check for yourself:  ls -la ~/.gradle ~/.android ~/Library/Android 2>&1
NOTE
}

# The real home, remembered before android_env() redirects it, so doctor can
# look at the place we are promising not to touch.
REAL_HOME="$HOME"

case "${1:-}" in
  toolchain) cmd_toolchain ;;
  init)      cmd_init ;;
  doctor)    cmd_doctor ;;
  devices)   cmd_devices ;;
  install)   cmd_install ;;
  dev|run)   cmd_dev ;;
  build)     cmd_build ;;
  logs)      cmd_logs ;;
  shell)     cmd_shell ;;
  env)       cmd_env ;;
  clean)     cmd_clean ;;
  *) sed -n '3,25p' "$0" | sed 's/^# \{0,1\}//' ; exit 1 ;;
esac
