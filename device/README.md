# Device configuration for the Miyoo Flip

Files here are applied to the handheld, not built into anything — but most of
them are now **also compiled into `moose-patch`**, which applies and reverts
them itself. See `docs/knulli-addon.md`.

Read this file for *why* each one goes where it goes; the app is where the
doing lives. Anything applied by hand from here uses `## Moose Rack:` markers, and
the app uses `## moose-patch:` — two sets of the same settings in one config
is last-wins and unreadable, so pick one.

    moose-patch --status     what every patch is currently at
    moose-patch --restore    put a device back to the saved profile
    moose-patch --save       write down what it is at now

## `flip-hotkeys.conf`

Appended to `/userdata/system/knulli.conf`. Everything is **Hotkey** (the Menu
button) plus the named button:

| Press | Does |
|---|---|
| B | Exit RetroArch — **twice**, so it cannot happen by accident |
| X | RetroArch menu |
| Y | Toggle shader |
| A | Show FPS |
| L1 / R1 | Load state / Save state |
| L2 / R2 | Pause / Fast-forward (hold) |
| D-pad ← → | Previous / next save-state slot |
| D-pad ↑ ↓ | Previous / next shader |

Set here rather than inside RetroArch because **configgen rewrites
`retroarch.cfg` at every launch** — which is why changes made in RetroArch's own
menu did not stick. `knulli.conf` lives on the persistent partition and is
applied on top at each start.

The stock layout had **four double bindings**, and one of them was dangerous:
Hotkey+A was `reset` *and* `fps_toggle`, so there was no way to see the frame
rate without restarting the game. Y was save-state *and* shader-toggle; L2 and
R2 each did two things. The five now bound to `nul` at the bottom of the file
are the leftovers of those pairs.

## `flip-shaders.conf`

Also appended to `/userdata/system/knulli.conf`.

`sharp-shimmerless` is the shader that matters on this screen. At 640×480 none
of these systems divide evenly — SNES is 256×224, so 2× leaves 32 rows over —
and the usual fix, bilinear, smears everything. Shimmerless keeps the pixel
grid even and only softens the seam. The `-lcd-crt` set applies it everywhere
and adds an LCD grid on gb/gbc/gba/gamegear, scanlines on the consoles.

Everything in the set is one pass except the handheld preset, which is two.

## `retroarch-shaders/`

Copied to `/userdata/system/configs/retroarch/shaders/`, which is RetroArch's
`video_shader_dir`. **Hotkey + D-pad ↑/↓** cycles the presets in that
directory, so these three are the alternatives reachable in-game: plain
shimmerless, shimmerless with scanlines, and shimmerless with the LCD grid.

They spell out absolute paths because RetroArch resolves a preset's shader
paths relative to the preset, and these no longer sit beside their `.glsl`
files.

## `gba-bezel/`

Copied to `/userdata/decorations/moose-rack/`, then selected with `gba.bezel=moose-rack`.

KNULLI's own GBA bezel renders the wordmark in a washed-out khaki-olive
gradient. This is the same artwork and the **same geometry** — the game keeps
the full 640 width and the 84px apron at the bottom carries the label — with
the wordmark recoloured to the console's metallic silver.

It lives under `/userdata` because `/` is an overlay whose writable layer is a
256 MB tmpfs: anything written to `/usr` is gone at the next boot.

Choosing bezels per system, from a GUI, is parked.

## `hotkey/`

`multimedia_keys.append` is appended to a copy of KNULLI's own
`multimedia_keys.conf`, saved as `/userdata/system/configs/multimedia_keys.conf`.
`S50triggerhappy` prefers that path over anything in `/etc` — which matters,
because `/etc` is on the tmpfs overlay and would be back to stock at the next
boot. `scripts/knulli.sh install` does all of this.

**L2+R2** runs `scripts/moose-hotkey.sh`. It has to be a two-button combo whose
trigger is the *second* button: triggerhappy matches on the exact set of held
buttons, so a rule on Menu alone would fire the instant Menu went down, before
the second button of any Menu combo was pressed.

The chain is `moose-hotkey.sh` → `moose-rack-launch.sh` → `moose-sdl`. The hotkey
script is the part that deals with not having been launched by
EmulationStation: it bails out if a game is running, stops ES to get the
display — ES holds DRM master and only drops it for emulators *it* starts —
and restores ES from a trap, so a crash in the app cannot leave the device on
a black screen. ES is restarted through its init script rather than by hand,
because started by hand it comes up without `XDG_RUNTIME_DIR` and has no sound.

ES's own L2/R2 navigation is turned off by `es_input.cfg` — **just the Flip's
own entry**, 24 lines, with `l2` and `r2` dropped.

Copying the shipped file across whole does not work. It holds 291 pad
definitions and 412 KB, and ES will not start with it as the user config: it
exits before writing a line to its log. A user `es_input.cfg` is meant to hold
the pads ES has been told about, not the catalogue it ships with.

## `splash/`

This is the fix for the KNULLI beetle that appeared on **every game launch and
every exit**.

It is not a setting, and it is not RetroArch or the framebuffer — both of which
I chased first and both of which were wrong. EmulationStation draws
`/usr/share/emulationstation/resources/logo.png`, a 1280×720 image of the
beetle with KNULLI under it, whenever it is loading. Launching a game and
returning from one are exactly when that happens. There is no option for it:
the file is referenced once in the ES binary and drawn unconditionally.

So `blank-logo.png` — 1280×720, black, **RGB not RGBA** — is copied over it.
Same size and same mode as the file it replaces, because a resource ES loads
unconditionally is not the place to find out whether it minds an alpha
channel.

`boot-custom.sh` goes to `/boot/boot-custom.sh`, which runs as `S00bootcustom`.
It has to be that early: `/usr` is on the tmpfs overlay and is stock again at
every boot, and ES starts at `S31`, so the swap must happen before then. The
same hook also re-applies the chosen Mali driver; the GPU half was there first
and its early `exit 0`s would have skipped anything appended after it, so both
are functions now.

Reverting is deleting the hook — the real logo is on the read-only squashfs and
comes back by itself.

`custom.sh` is appended to `/userdata/system/custom.sh`, run by
`S99userservices`. It zeroes `/dev/fb0`, where `S03system-splash` leaves the
boot logo. That is **not** what was causing the flash — the framebuffer was
verified black across all three buffers while the logo was still appearing —
but it is what shows if anything stops drawing, so it is worth keeping and the
app's launcher no longer has to do it itself.

## Restarting EmulationStation over SSH

`/etc/init.d/S31emulationstation start` runs `emulationstation-standalone &` —
backgrounded from *your* shell. Over SSH that means the session is its parent,
and when the command returns, SIGHUP takes ES with it. It looks exactly like ES
crashing a minute after it starts, and it cost most of an afternoon.

    . /etc/profile.d/xdg.sh; . /etc/profile.d/dbus.sh
    setsid /usr/bin/emulationstation-standalone </dev/null >/dev/null 2>&1 &

`setsid` is the whole difference. Sourcing the two profiles is the other half:
started without them ES comes up with no `XDG_RUNTIME_DIR`, cannot reach
PipeWire, and has no sound. `scripts/knulli.sh` has done this correctly all
along — `setsid nohup` at line 260.

Starting it more than once leaves two respawn loops, and two EmulationStations
fight over DRM so neither survives. Check with `ps -e -o args= | grep -c
'^emulationstation '` — and note that a `grep` for the *wrapper* matches its
own command line, so that count reads one too high.

## `flip-power.conf`

`system.batterysaver.extendedmode=none`. The device suspends itself after 15
minutes idle, which drops the network and reads as a dead device. Dimming stays.
