# The handheld OS — base, decision, and build plan

The device is the **Miyoo Flip** (RK3566, quad A55 @ 1.8 GHz, Mali-G52 2EE,
1 GB LPDDR4, 3.5" **640×480**, clamshell, ~3000 mAh). This file is the OS/base
decision. `handheld-frontend.md` is the front end that runs on it.

Prior device work (dArkOS / MiniLoong) was removed on 2026-08-23 — that device
was returned. Do not reintroduce it.

## The decision

> **SUPERSEDED 2026-08-24 — base is now KNULLI, not ROCKNIX.** ROCKNIX didn't
> boot; KNULLI did, and on-device probing reversed the deep-sleep call. The
> ROCKNIX reasoning below is kept for history; the live decision is in the
> **"Update 2026-08-24 — base switched to KNULLI"** section at the end of this
> file. Note KNULLI-Flip is BSP 5.10, so the "modern kernel" framing here no
> longer applies.

**Approach 3B: roll our own minimal system, starting from a
working distro image rather than from scratch.** We fork a known-good base,
strip it to the parts we reuse, and add our own front end.

**Base: Zetarancio's ROCKNIX Flip work.**
- Reference/manual: `reference/Miyoo-Flip-Mainline-Linux-Reverse-Engineering`
  (GPL v2, a full documented teardown — boot chain, DTS, drivers, flashing,
  recovery). This is the thing that lets us maintain the kernel ourselves if
  the maintainer stops, which is the whole reason 3B is survivable.
- Buildable images: `Zetarancio/distribution` branch `flip`, kernel **7.0.2**.

### Why this base, not the others

| Option | Verdict |
|---|---|
| Overlay on stock kernel (spruce/NextUI) | Durable vendor kernel, but old 5.10 BSP and a large sysfs "hardware layer". Fallback if 3B's suspend can't be made efficient — see Q4. |
| Full distro kept whole (KNULLI/ROCKNIX) | We're replacing the front end, so the kitchen-sink userland is mostly waste. |
| **3B on Zetarancio (chosen)** | Modern kernel, documented well enough to own, near-complete hardware support. Single-maintainer risk is mitigated by the RE repo being a real manual. |
| 3B from debootstrap | Truest "pure Linux + apt", but we'd bring up boot/init ourselves. Possible second pass; not the starting point. |

### Rejected shortcuts, for the record
- **Pin a modern kernel onto spruce** — impossible. Spruce *is* its stock-kernel
  coupling: its WiFi/BT `.ko` modules are built for 5.10, its libmali is paired
  to the BSP kbase, its sysfs paths and DRM property IDs are BSP-specific. Swap
  the kernel and every kernel-coupled spruce tool breaks; what survives
  (RetroArch, emulators, ports) runs on any kernel and never needed spruce.
- Kernel + its hardware-layer userland are **one welded unit**. Only the
  application userland (RetroArch, cores, PortMaster, standalone emulators) is
  portable across kernels.

## A Wayland compositor does run on the Flip (2026-08-25)

KNULLI ships the **GBM-only, no-Wayland** build of the Mali blob, and that alone
is why no compositor starts — not the GPU, not the kernel. Swapping in
`libmali-bifrost-g52-g24p0-wayland-gbm.so` (same vendor driver, different build)
brings up both Weston and Sway with GLES 3.2. Emulators are unaffected: same
speed, and byte-identical rendered frames.

Full account, with reproduction: **`docs/flip-wayland-and-the-gpu-blob.md`**.
Staging script: `scripts/flip-mali-shim.sh`.

This is what makes a WebKitGTK — and therefore a Tauri — build worth attempting
on this device at all.

## Hardware status on the base (RE repo, kernel 7.0.2, 2026-07)

Working: boot, display, backlight, audio (PipeWire + rk817), WiFi (RTL8733BU),
Bluetooth, **GPU (mali_kbase r54p2 + libmali g24p0 blob — NOT Panfrost)**,
storage + both SD slots, HDMI + HDMI audio, DDR scaling (out-of-tree DMC),
VPU/RGA, all 17 buttons + joypad + rumble, **standard suspend**.

Not fully working:
- **IEP** — video post-processing block, BSP-only. Irrelevant to gaming. Ignore.
- **Deep suspend** — deferred; the driver (`rk3568-suspend`, patches 1013a/1013b)
  exists but is `*.testing-disabled` and `CONFIG_RK3568_SUSPEND_MODE` is off.
  **Blocked by an upstream EmulationStation bug — i.e. by the front end we are
  removing.** See Q4; this is the one real risk on this base.

GPU discipline: **pin both `mali_kbase` r54p2 and `libmali` g24p0**; they are a
matched pair. Panfrost matches the same DT compatible and must stay blacklisted.

## The build model (LibreELEC-style)

ROCKNIX is LibreELEC-lineage: **read-only SYSTEM squashfs** + **writable
`/storage` overlay**, **systemd**, front end as a service. This gives us the
update-survival property for free: the OS is a sealed image, our stuff lives on
`/storage` and survives an image reflash. Two places to modify:
1. **Runtime overlay** (`/storage`) — drop binaries, config, a systemd override.
   Fast, reversible, no rebuild. Start here.
2. **Image build** (fork `distribution`) — change the package set, patches,
   enabled services. Produces a new SYSTEM squashfs. Do this once the runtime
   shape is settled.

## Approach — the brainstorm (answers to open questions)

### Q1 — Modify how?
Two edits, smallest first:
- **Swap the front end service.** Disable ES's unit, enable ours. One unit file.
  This is the only *required* change to make the device ours.
- **Add our binaries** (front end + RomM app) to `/storage`.
Later, at image-build time: trim the package set (Q3) and enable deep suspend
(Q4). Prove it all on the runtime overlay before rebuilding the image.

### Q2 — What we supply
- Our SDL/Rust front end (`src-sdl`, built for aarch64).
- The RomM app / sync engine (this repo's core: `savesync`, `statesync`, the
  `/api/*` client), as a static binary.
- Config: server URL + token (do not make anyone type a token on the device),
  control map, layout.
- Launch glue: how the front end starts a game — reuse ROCKNIX's per-emulator
  launch scripts, or call RetroArch directly.
- A front-end systemd unit (mirror ES's: `After=`, DRM env, `Restart=on-failure`).
- UI assets (fonts incl. CJK, icons, theme).
See Q5 — the full list depends on scope.

### Q3 — What we take away, and whether keeping it is a hazard
- **Must disable: the ES service.** Not because the files are dangerous, but
  because two graphical programs cannot both own the KMSDRM display — leaving
  ES autostarting fights our front end for the screen. Disabling the *service*
  is enough; deleting the *files* is optional cleanup, not safety.
- **Everything else is redundant, not hazardous.** Unused cores, ES themes,
  media tools — they cost disk and nothing else; we never open their menus.
- Net: the required removal list is exactly one item (the ES autostart). The
  rest is taste and disk space.

### Q4 — Will deep suspend bite us? (the real risk)
Honest picture on this battery-poor clamshell:
- **Stock BSP supports deep suspend** natively → ~100–120 h standby.
- **This base ships with deep suspend OFF** → ~40–50 h standby. So out of the
  box, our base is ~2× worse in sleep than stock. Active-play battery is
  governed by DVFS/DMC, which works, so playing time should match stock; the
  gap is standby only — but a clamshell is closed and sleeping most of its life,
  so standby matters here.
- **It is recoverable.** The deep-suspend driver exists in-tree; it's disabled
  to protect ES users, and we're removing ES. We can enable
  `CONFIG_RK3568_SUSPEND_MODE`, un-disable patches 1013a/1013b, set `vdd_logic`
  `regulator-off-in-suspend`.
- **Not guaranteed.** It's `testing-disabled` = unvalidated resume stability. We
  own that testing. If we cannot make it stable, we're stuck at ~40–50 h, which
  is a genuine reason to reconsider the stock-kernel path (3a) for this device.
- **Action: validate deep suspend early**, before building much on top. It is
  the single finding most likely to send us back to stock.

### Q5 — What to supply is open-ended (scope options)
It is a product-scope decision. The axes:
- **Minimal RomM handheld:** our front end + RomM sync + RetroArch + the cores
  we care about. Nothing else.
- **+ Ports:** PortMaster (native games/ports).
- **+ Standalone emulators:** PPSSPP, DraStic, etc. for systems RA does poorly.
- **+ Enrichments:** RetroAchievements, netplay, shaders/bezels.
- **+ Media/utility:** music/video player, ebook reader.
- **+ Remote/dev:** SSH, Samba, Syncthing (wireless dev loop, file access).
- **Front-end depth:** library-only vs. also settings, per-game emulator config,
  save/state management, and the RomM sync tab (status / manual push-pull of
  saves and states to the server).
Default starting scope: **minimal RomM handheld + SSH for the dev loop**, grow
from there.

## First step (do this before building anything on top)
Build the stock `Zetarancio/distribution` `flip` image **unmodified**, flash it,
confirm it boots on our unit and does not freeze the way ROCKNIX did before.
Then immediately test **standard suspend** and attempt **deep suspend** (Q4).
That validates the foundation and the one real risk in one sitting.

## Boot / recovery facts (from the RE repo)
- Boot chain: BootROM → preloader (SPI NAND 0x0) → U-Boot FIT (**must include
  OP-TEE BL32**) → kernel. SD boot = erase/zero the preloader so BootROM falls
  through.
- Stock ↔ ROCKNIX without opening the device: **Preloader Eraser** app (stock→SD),
  `write-preloader-mtd.sh` + `preloader.img` (SD→stock). Both in the RE repo.
- Not brickable: BootROM + MASKROM live in SoC ROM, not SPI. Worst case is
  MASKROM + `xrock` recovery. **Back up the full SPI NAND before touching it.**

---

# Update 2026-08-24 — base switched to KNULLI (verified on-device)

ROCKNIX did **not** boot on the unit (bad ROCKNIX image, not a device issue).
**KNULLI (Batocera) boots fine and is now the working base.** SSH'd in at
`10.10.10.187` (`root` / `linux`) and probed it. This overturns two earlier
assumptions — recorded honestly below.

## What KNULLI-Flip actually is (measured)
- **Batocera.linux 42**, Buildroot 2024.11.
- **Kernel: Linux 5.10.209 — a BSP kernel, NOT mainline.** (Earlier I assumed
  KNULLI ran mainline like ROCKNIX. Wrong.) So KNULLI-Flip rides the **vendor
  BSP 5.10 lineage** — durable, no single-maintainer kernel bus-factor, but the
  hardware layer is BSP-style (raw sysfs / `modetest`-class, like stock), and
  the RE repo's **stock/BSP** docs apply, not its mainline patches.
- **GPU:** `mali_kbase g18p0` BSP blob; `/dev/mali0`, `/dev/dri/card0`+`card1`,
  `renderD128/129`. Not Panfrost.
- **Init: NOT systemd** — Batocera `S##` init.
- **Deep sleep: `/sys/power/mem_sleep = s2idle [deep]`, `state = freeze mem`.**
  Deep is available and default (inherited from the BSP path). **This reverses
  the earlier Q4 call** — KNULLI likely has GOOD standby, unlike ROCKNIX where
  mainline deep-sleep was *deferred*. (Actual drain still to be measured; the
  device dropped off WiFi by auto-suspending, which is corroborating.)

## Storage model (immutable OS + persistent /userdata)
- OS = read-only squashfs `/dev/loop0` → `/overlay/base` (1.9 G).
- `/` = **tmpfs overlay upper → changes to `/` are lost on reboot.**
- **`/userdata` = `/dev/mmcblk1p4`, 235 G, persistent, mode 0777.** Our stuff
  lives here and survives OS-image updates.
- Boot partition `/dev/mmcblk1p3` (4 G FAT, `/boot`, holds `knulli-boot.conf`).
- `/userdata` dirs: `bios cheats decorations roms saves screenshots system themes`.

## Frontend swap (two clean options)
- ES autostart: `/etc/init.d/S31emulationstation` → `/usr/bin/emulationstation-standalone`
  (sets `HOME=/userdata/system`, reads `/boot/knulli-boot.conf`, reboots ES on
  stop/crash — so a bare `killall` restarts it, like spruce's loop).
- **(a) `/userdata/system/custom.sh`** — Batocera boot hook, run by
  `S00bootcustom`. Persistent, survives updates. Preferred for dropping our
  frontend in without touching the image.
- **(b)** disable/replace `S31emulationstation`.

## Reusable toolset (big shortcut for the hardware layer)
99 libretro cores in `/usr/lib/libretro`; RetroArch at `/usr/bin/retroarch`;
full Batocera system set under `/userdata/roms`. And a large set of **`knulli-*`
/ `batocera-*` helper CLIs that already wrap the hardware/config layer** — call
these instead of writing raw sysfs pokes:
`knulli-brightness`, `knulli-battery-check`, `knulli-battery-hud`, `knulli-audio`,
`knulli-mixer`, `knulli-config`, `knulli-cores`, `knulli-display-settings`,
`knulli-overclock`, `knulli-fan-control`, `knulli-mount`, `knulli-info`,
`knulli-board-capability`, `knulli-bluetooth`, `knulli-format`, `knulli-install`,
`batocera-evmapy` (input remap), `batocera-test`, `batocera-makepkg`,
`batocera-moonlight`. This shrinks our platform module: shell out to these where
they exist; only drop to sysfs for what they don't cover.

## Dev loop
- SSH `root`/`linux` works. **Key auth is blocked by `StrictModes`** because
  `/userdata` and `HOME=/userdata/system` are 0777 — to get passwordless keys,
  set `StrictModes no` (or use a properly-permissioned home). Password-via-expect
  works meanwhile.
- Wireless loop confirmed: `ssh` + `rsync` to `/userdata`. No card removal.
- SSH hygiene note: kill stray local ssh sessions between runs; a runaway remote
  command (e.g. `grep -r /sys`) will wedge new logins.

## Still to probe — closed 2026-08-24
The four hardware items below were answered on-device; see **Hardware layer**.
The two remaining Step 0 items (`ls /userdata/roms`, RetroArch paths) were
measured on the device the same day — see *Step 0, closed on the device* in
`port-plan.md`. Nothing is outstanding.

## Decision update
**Base = KNULLI (Batocera 42, BSP 5.10), verified working.** Trade vs the old
ROCKNIX pick: give up the mainline kernel (bigger BSP hardware layer) and
systemd (use `custom.sh` instead); gain: **it boots, deep sleep is available,
biggest toolset, durable vendor-lineage kernel, easy releases, GammaLoader-proven
dual-boot.** ROCKNIX/Zetarancio stays the **kernel/hardware reference** (the RE
repo), not the runtime base.

---

# Hardware layer — KNULLI (BSP 5.10), measured on-device 2026-08-24

The platform-module facts for our front end. Node names are the KNULLI/BSP 5.10
tree's; the RE repo's stock/BSP docs are the reference for these.

## Input (`/proc/bus/input/devices`)
| Device | Node | Role |
|---|---|---|
| `Miyoo Flip Controller` | `js0` / **`event5`** | gamepad — buttons + sticks (our main input) |
| `hall wake key` | **`event1`** | **lid** (hall sensor) — open/close |
| `rk805 pwrkey` | `event2` | power key |
| `gpio-keys-polled` | `event0` | polled GPIO keys (volume, etc.) |
| `rockchip-rk817 Headset` | `event3` | headphone jack detect |
| `hdmi_cec_key` | `event4` | HDMI-CEC |

So: read the pad from **`/dev/input/event5`**, the lid from **`event1`** — a
proper input device here (nicer than stock's raw `hall-mh248` sysfs read),
though the platform node `/sys/devices/platform/hall-mh248` also exists.

## Backlight
- `/sys/class/backlight/backlight/` — **`brightness` 0–255** (`max_brightness`=255).
- Prefer the wrapper **`knulli-brightness`** over poking sysfs directly.

## Power / battery (`/sys/class/power_supply/`)
- Nodes: **`ac`**, **`battery`**, **`usb`**.
- `battery/capacity` = percent (read 33 mid-charge), `*/status` = `Charging`.
- Charge state: `ac/online` or `usb/online`; helper **`knulli-battery-check`**.
- Banner extras: battery reads **87% / 3.93 V** but **"Battery Calibrated: No"**
  — the gauge is uncalibrated, so percentages may be rough until calibrated.

## System (from login banner)
- Board `miyoo-flip`, Linux **5.10.209**, 4× A55, **max 1992 MHz**, 640×480@60,
  ~970 MB RAM, idle temp ~51 °C, OS `scarab 2026/05/10`.
- **`/userdata` is exfat, ~155 G free.** (exfat, not ext4 — fine for our data,
  but note no POSIX perms/symlinks, like FAT.)

## Wi-Fi, audio and the clock (measured 2026-08-25)

For the status corner and the Device settings pane.

| | |
|---|---|
| Wi-Fi helper | **`knulli-wifi`**, with `scanlist` and `list` subcommands. There is **no `batocera-wifi`** on this image. |
| Signal, cheaply | `/proc/net/wireless` — one row per interface, third column is link quality out of 70. A plain file read, so the corner can poll it. |
| Signal, fully | `iw dev wlan0 link` gives SSID and dBm. A process, so settings only, never per frame. |
| Also present | `wpa_cli`, `connmanctl` |
| Audio | `knulli-audio`, `knulli-mixer`, `amixer` |
| Clock | `date` is correct and `/etc/timezone` reads `America/New_York`, so the device knows its own zone — `localtime_r` is enough, no date library needed. |

Read at the time: `wlan0`, quality **40 of 70**, connected to `Chicken24` at **-52 dBm**.

## The Wi-Fi helper's actual interface (read off the device 2026-08-25)

`/usr/bin/knulli-wifi` is a 124-line shell script over `connmanctl`. Its usage
text lists only two subcommands; the case statement has seven.

| | |
|---|---|
| `knulli-wifi list` | networks, **one plain SSID per line**. No columns, no signal. Names contain spaces and hyphens — `DIRECT-AB-HP OfficeJet Pro 6970` — so take the whole line. |
| `knulli-wifi scanlist` | `connmanctl scan wifi`, `sleep 1`, then `list`. |
| `knulli-wifi enable <ssid> <key>` | the join. Writes `wifi.ssid` / `wifi.key` through `knulli-settings-set`, reloads connman, then **waits up to 20 s** for an address. Not a call to make on a draw loop. |
| `knulli-wifi disable` | turns the radio off and waits for the address to go. |
| `knulli-wifi get_route` | default gateway via a `wl*` interface — a cheap "am I actually on" check. |
| `knulli-settings-get wifi.ssid` | the saved network. Read `Chicken24`. |

The password is connman's to keep. It is passed to `enable` and never stored by
us — a copy in our own config would be a plaintext password on an exfat card
with no permissions.

## Ports and Tools — they are groups, not folders (2026-08-25)

The first reading of this was wrong twice, and both mistakes came from assuming
a structure rather than reading `es_systems.cfg`.

**They are `<group>`s.** `/userdata/roms/tools` holds nothing but `_info.txt`;
the **Tools group** is `odcommander` and `vaixterm`, each its own system with
its own directory. The **Ports group** is nine populated systems: `abuse`,
`cannonball`, `iortcw`, `mrboom`, `ports`, `prboom`, `sdlpop`, `superbroswar`,
`xrick`. `emulators` is its own group holding PPSSPP and ScummVM.

**Every member has its own extensions.** `.sh .squashfs` is only the `ports`
system's. The others use `.odc`, `.vxt`, `.wad .iwad .pwad`, `.game`, `.rtcw`,
`.libretro`, `.sdlpop`, `.cannonball`, `.zip`. A scan looking for shell scripts
finds one system in ten.

| | |
|---|---|
| Metadata | `gamelist.xml` per system — `<path>` (relative, `./`), `<name>`, `<image>` |
| Launch | `emulatorlauncher -system <member system> -rom <path>`. Its argparse marks exactly those two `required=True`. |

Note the system is the **member**, not the group: launching Doom passes
`-system prboom`, not `-system ports`.

## The Wi-Fi helper's actual interface (read off the device 2026-08-25)

`/usr/bin/knulli-wifi` is a 124-line shell script over `connmanctl`. Its usage
text lists two subcommands; the case statement has seven.

| | |
|---|---|
| `knulli-wifi list` | networks, **one plain SSID per line**. Names contain spaces — `DIRECT-AB-HP OfficeJet Pro 6970` — so take the whole line. |
| `knulli-wifi scanlist` | `connmanctl scan wifi`, `sleep 1`, then `list`. |
| `knulli-wifi enable <ssid> <key>` | the join. **Waits up to 20 s** for an address. Not a call to make on a draw loop. |
| `knulli-wifi disable` | radio off. |
| `knulli-wifi get_route` | default gateway — a cheap "am I on" check. |
| `knulli-settings-get wifi.ssid` | the saved network. |

The password is connman's to keep. It is passed to `enable` and never stored by
us — a copy in our own config would be a plaintext password on an exfat card
with no permissions.

## Front-end platform module — what to wire
- Input: `/dev/input/event5` (pad), `event1` (lid), `event0` (volume keys).
- Brightness: `knulli-brightness` (or `/sys/class/backlight/backlight/brightness`).
- Battery/charge: `battery/capacity`, `ac|usb/online` (or `knulli-battery-check`).
- Sleep: `echo mem > /sys/power/state` (deep available — see above).
- Reuse `knulli-*` wrappers first; drop to sysfs only where none exists.

**Hardware map complete — nothing left pending.**

## SSH note (for the dev loop)
`root` / `linux` works. Interactive login is instant. Automated
`ssh host "cmd"` **must use `-tt`** (force a PTY) or KNULLI's login flow stalls;
and don't fire rapid retries. Key auth needs `StrictModes no` (/userdata is
world-writable) — optional; password is fine.

**Do not set up keys — drive the password with `expect`.** This works first
time and is the whole loop:

    #!/usr/bin/expect -f
    set timeout 45
    log_user 0
    spawn /usr/bin/ssh -tt -o StrictHostKeyChecking=accept-new \
      -o UserKnownHostsFile=/dev/null -o PreferredAuthentications=password \
      -o PubkeyAuthentication=no root@10.10.10.187 [lindex $argv 0]
    expect -re {[Pp]assword:} { send "linux\r" }
    log_user 1
    expect eof

The device is **`10.10.10.187`**, and it answers to **`knulli.local`** over
mDNS, so `dscacheutil -q host -a name knulli.local` finds it again if the lease
moves. Strip the trailing `Connection to … closed.` line from any output you
parse.

---

# Emulator/core choice — the principle (2026-08-24)

**Align with moose-rack's emulators where the device can actually run them.
Always check what the CPU and GPU can do before picking; if alignment costs
emulation speed, speed wins.** Alignment is a preference, not a rule — it is
only worth having when the game still runs full speed on the handheld.

**Exception: Neo Geo stays on `geolith`.** The roms here are geolith-specific,
so the core is fixed by the library, not by performance. Do not "fix" it to
FBNeo.

For this device that means checking picks against the Flip's real hardware:
4× A55 @ 1.8 GHz, Mali-G52 2EE, 1 GB RAM, 640×480. A desktop-grade core that
assumes a modern x86 CPU (SwanStation, full Flycast) will run, badly.

## The trap: KNULLI's configgen does not validate core names

`/userdata/system/knulli.conf` holds `<system>.core=` / `<system>.emulator=`.
**configgen takes those strings verbatim — no validation, no fallback.** A name
that doesn't exist is not an error at config time; it becomes a failed launch.
And **moose-rack's core names are not KNULLI's** — this is what broke the
first hand-written block:

- `genesis_plus_gx` (moose-rack) vs **`genesisplusgx`** (KNULLI, the actual
  `.so` name).
- `mupen64plus_next` is a *libretro core*; the N64 default emulator here is
  **standalone `mupen64plus`**, which takes a *video plugin* — `rice`,
  `glide64mk2`, or `gliden64`. Mixing the two namespaces produces a config that
  cannot launch.

Check a name two ways before writing it:
- `ls /usr/lib/libretro/<core>_libretro.so` — the core exists.
- `<name>` appears in `/usr/share/emulationstation/es_systems.cfg` for that system.

Then verify what the config actually resolves to, without launching a game:

```sh
python3 -c "import sys,glob;[sys.path.append(p) for p in glob.glob('/usr/lib/python3*/site-packages')];
from configgen.Emulator import Emulator;
print(Emulator(name='n64', rom='/tmp/x').config)"
```

## KNULLI already tuned this device — read its defaults first

Two files, the second overriding the first **for this hardware**:
- `/usr/share/knulli/configgen/configgen-defaults.yml` — generic defaults.
- `/usr/share/knulli/configgen/configgen-defaults-arch.yml` — **per-device**.

The arch file is where the low-power picks live: `dreamcast: flycastvl` (the
low-spec Flycast, not full `flycast`) and `n64: mupen64plus` + `rice`. Overriding
these with the generic names silently gives up the device tuning. **Start from
the arch defaults and deviate only with a reason.**

## The settled set (verified resolving on-device 2026-08-24)

| System | Emulator / core | Note |
|---|---|---|
| psx | libretro / `pcsx_rearmed` | was `swanstation` — far too heavy for A55s |
| megadrive | libretro / `genesisplusgx` | was `genesis_plus_gx` — **core file did not exist** |
| n64 | **`mupen64plus`** / `rice` | was libretro `mupen64plus_next` — not a valid plugin; `glide64mk2` looks better if the speed is there |
| dreamcast | libretro / `flycastvl` | was `flycast` — arch default is the low-spec build |
| neogeo | libretro / `geolith` | **fixed by the roms**, not by speed |
| nes | libretro / `fceumm` | KNULLI default |
| snes | libretro / `snes9x` | KNULLI default; keeps SuperFX/SA-1 right |
| gb, gbc | libretro / `gambatte` | KNULLI default |
| gba | libretro / `mgba` | KNULLI default |
| fbneo | libretro / `fbneo` | KNULLI default |

Backups of the pre-change config: `/userdata/system/knulli.conf.bak-psx`,
`knulli.conf.bak-cores`.

## Applying a core change
Edit `knulli.conf`, then **restart ES** (`killall emulationstation`; the
`S31emulationstation` wrapper brings it back). ES holds the config in memory and
writes the whole file back when settings change — without the restart, an edit
can be silently reverted.

## Saves are per system, not per core
`/userdata/saves/<system>/` is keyed by **system**, so switching cores does not
move or orphan saves. PSX memory cards are raw 128 KB cards (`MC` magic) and
carry across PCSX-ReARMed/SwanStation untouched. **Save states do not** — they
are core-specific and never transfer. `.ldci` files are SwanStation's disc-swap
bookmarks; PCSX-ReARMed ignores them.

## Global settings checked, all clean
Rewind, shader sets, bezels/decorations, and smoothing are all at defaults
(commented out) — none of them are taxing the device. Governor is `schedutil`.
