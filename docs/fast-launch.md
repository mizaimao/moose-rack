# Making games start faster

Measured on the Flip, 2026-08-28. A warm GBA launch takes **4.26 s**, and the
emulator is not the problem.

| Phase | Time |
| --- | --- |
| Python interpreter + imports | 1.08 s |
| evmapy kill-and-wait | 0.93 s |
| config merge, controller map, generator, bezels | ~1.42 s |
| **`configgen` total** | **3.43 s** |
| RetroArch → first frame | 0.83 s |

RetroArch loads a core, a ROM and ten shader passes in 0.83 s. Everything else
is `configgen`: 272 Python files, imported from nothing on every launch, to
produce one 114 KB text file and an argv.

## 1. evmapy — done, 0.93 s

`batocera-evmapy start` kills the daemon, touches a flag, starts it again and
blocks on `inotifywait` for the flag to come back. It is a process round trip,
not work.

`/usr/share/evmapy/libretro.keys` declares **only** `actions_gun1` — a lightgun
combo. configgen writes a per-device `.json` into `/var/run/evmapy` only when
the merged keys file has an `actions_playerN` for that pad, so a libretro
launch with no lightgun writes none, and then waits 0.93 s for a daemon with
nothing to do.

The guard is one line, and it does not need to know what libretro is:

```sh
ls /var/run/evmapy/*.json >/dev/null 2>&1 || exit 0
```

`__prepare()` writes the JSON *before* `__enter__` calls `start`, so "no device
config" is exactly "nothing to map". The other 54 `.keys` files — flycast,
amiberry, hatari, azahar, gsplus — all declare real player mappings and are
unaffected. **An unconditional stub would have broken every standalone
emulator**, which is what a first A/B of this did.

Measured: 3.43 s → 2.50 s, three runs each side.

Persistent as `launch-evmapy` in the moose-patch catalogue. `/usr` is an
overlay on a 256 MB tmpfs, so `boot-custom.sh` reinstalls the line at every
boot from a flag on `/boot` — the same mechanism as `es-logo` and `gpu`, and
on `/boot` for the same reason: the hook runs at S00 and `S02resize` is what
mounts `/userdata`. Turn it off with `./moose-patch --apply launch-evmapy=off`.

## 2. `moose-fastlaunch` — the launcher, in Rust

`src-fastlaunch/`. Does configgen's job natively for the systems actually
played here, and hands everything else back.

Twelve systems have real game counts, all libretro, eleven with a core pinned:

    fbneo 2504 · nes 1219 · snes 982 · megadrive 943 · gba 774
    gb/gbc 1109 · neogeo 162 · psx 108 · n64 43 · dreamcast 28

So this is not a rewrite of configgen's 183 systems. It is a fast path plus a
fallback.

### Why this rather than caching

Caching configgen's *output* was the other option, and it can go stale
silently: wrong core, wrong controls, on one game, noticed weeks later.
Computing natively is the same ceiling with none of that — it does not
remember anything, it is just quick.

### The safety story

One rule: **when in doubt, `exec` the Python launcher.** Unknown system,
non-libretro emulator, core not on disk, unreadable `knulli.conf`, a core name
containing a `/` — all end in `fall_back()`, which costs the 3.4 s we were
saving and is otherwise indistinguishable from not having installed it.

`exec` rather than spawn-and-wait, because EmulationStation is watching this
process and an extra one in the middle would mean forwarding its exit code and
signals correctly.

### Verified against the device

`--plan` resolves without launching. Against the real `knulli.conf`:

    gba -> vba-m          snes -> snes9x        nes -> fceumm
    megadrive -> genesisplusgx                  n64 -> mupen64plus-next
    dreamcast -> flycastvl                      neogeo -> geolith
    psx -> pcsx_rearmed   gb -> gambatte
    amiga500, ports, pygame -> fallback

### What is not done

Generating `retroarchcustom.cfg` — 114 KB, ~3,200 lines. That is the bulk of
the remaining work and it is deliberately not guessed at. The plan is
differential: generate with Python, generate with Rust, `diff`. It is a
deterministic text file, so correctness has a mechanical answer rather than a
confident opinion. Generation lands only when that diff is clean across every
system, and the same harness is what catches drift after a KNULLI update.

Until then the binary resolves and falls back — correct, not yet fast.

### Cross-compiling

    export CARGO_HOME=$PWD/.toolchain/cargo RUSTUP_HOME=$PWD/.toolchain/rustup
    export PATH=$PWD/.toolchain/cargo/bin:$PWD/.toolchain/zig-*:$PATH
    cargo zigbuild -p moose-fastlaunch --release --target aarch64-unknown-linux-gnu

398 KB. No dependencies — this runs on the launch path, so its own start-up
cost is the point.

## Ceiling

| | Saving | Total |
| --- | --- | --- |
| now | — | 4.26 s |
| evmapy guard *(done)* | 0.93 s | 3.33 s |
| native generation | ~3.3 s | **≈0.9 s** |

0.9 s is RetroArch alone, which is the floor without touching the emulator.
