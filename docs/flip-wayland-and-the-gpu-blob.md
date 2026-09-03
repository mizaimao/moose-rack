# A Wayland compositor on the Miyoo Flip

**Finding, 2026-08-25.** KNULLI on the Miyoo Flip ships a build of the Mali
driver with **no Wayland support and an old GBM ABI**, and that — not the
hardware, not the GPU, not the kernel — is why no compositor starts. Swapping in
a different build of *the same vendor driver* makes both Weston and Sway run,
with GLES 3.2, on the vendor stack. No Panfrost, no Mesa, nothing rebuilt.

This matters because a compositor is the thing WebKitGTK needs, and WebKitGTK is
the thing a Tauri build needs. The other half was already checked: Buildroot
carries webkitgtk 2.52.4 for aarch64 and its GTK3 path installs
`libexec/webkit2gtk-4.1/`, which is the API Tauri v2 requires.

## The symptom

Both compositors on the stock image fail, and they look like different faults:

```
weston: Failed to load module: /usr/lib/libweston-14/drm-backend.so:
        undefined symbol: gbm_bo_get_fd_for_plane
sway:   Segmentation fault
```

Sway's is a red herring twice over. It needs a seat manager, and the device runs
one (`seatd`, socket at `/run/seatd.sock`) — but `libseat` here has no `seatd`
backend compiled in, so `LIBSEAT_BACKEND=seatd` fails and only `builtin` works.
Given that, sway gets all the way to reading the display's EDID and then dies on
**the same missing symbol as Weston**, inside `libwlroots`.

## The cause

On this image GBM *is* the Mali blob:

```
/usr/lib/libgbm.so.1      -> libmali.so.1
/usr/lib/libEGL.so.1      -> libmali.so.1
/usr/lib/libGLESv2.so.2   -> libmali.so.1
/usr/lib/libgbm.so.1.0.0  -> libMali.so
```

Rockchip publishes libmali in many builds. The one here is the **GBM-only,
no-Wayland** build of an older release:

| | stock (`g13p0`) | `g24p0-wayland-gbm` |
|---|---|---|
| `gbm_bo_get_fd_for_plane` | absent | present |
| Wayland strings (`wl_display_`) | **0** | 14 |
| size | 43,388,472 | 56,358,648 |

`gbm_bo_get_fd_for_plane` arrived in Mesa 21.1. Weston 14 and wlroots 0.18 both
require it; this blob predates it. And with zero Wayland support compiled in,
even a client that got that far would have no EGL Wayland platform to bind.

Note what is *not* the cause: the kernel. `dmesg` reports
`Kernel DDK version g18p0-01eac0` while the userspace here is `g13p0` and the
one that works is `g24p0`. Both wiki and folklore say kernel and userspace must
be a matched pair; on this device a `g24p0` userspace ran fine on a `g18p0`
kernel.

## The fix

One file, from [ROCKNIX/libmali](https://github.com/ROCKNIX/libmali), which
mirrors Rockchip's builds:

```
lib/aarch64-linux-gnu/libmali-bifrost-g52-g24p0-wayland-gbm.so
```

The blob is a mega-library: EGL, GLES, GBM and Wayland-EGL in one file, reached
through symlinks. So an override directory needs every name the system points at
it.

## Reproducing it

Nothing below is installed. It is `LD_LIBRARY_PATH` for the processes started,
`/usr/lib` is untouched, and a reboot forgets all of it.

```sh
# 1. Fetch the blob (56 MB) and put it on the device.
curl -fL -o libmali.so.1 \
  https://raw.githubusercontent.com/ROCKNIX/libmali/master/lib/aarch64-linux-gnu/libmali-bifrost-g52-g24p0-wayland-gbm.so
scp libmali.so.1 root@FLIP:/tmp/mali24/libmali.so.1      # password: linux

# 2. On the device, give it the names the system expects.
cd /tmp/mali24
for n in libEGL.so.1 libGLESv2.so.2 libGLESv1_CM.so.1 \
         libgbm.so.1 libwayland-egl.so.1 libMali.so; do
  ln -sf libmali.so.1 $n
done

# 3. Take the display from EmulationStation. It has a respawn loop, so clearing
#    its flag is part of stopping it.
emulationstation-standalone --stop-rebooting
/etc/init.d/S31emulationstation stop
while pidof emulationstation >/dev/null; do sleep 0.5; done

# 4. Run a compositor against the new blob.
export XDG_RUNTIME_DIR=/tmp/wl-run; mkdir -p $XDG_RUNTIME_DIR; chmod 700 $XDG_RUNTIME_DIR
export LD_LIBRARY_PATH=/tmp/mali24
weston --backend=drm-backend.so --idle-time=0 &

# 5. And a client.
export WAYLAND_DISPLAY=wayland-1
weston-terminal &

# 6. Give the screen back.
pkill -x weston; cat /dev/zero > /dev/fb0
setsid nohup /usr/bin/emulationstation-standalone >/dev/null 2>&1 &
```

For sway instead of Weston, the same but:

```sh
export LIBSEAT_BACKEND=builtin   # the seatd backend is not compiled into libseat here
export WLR_RENDERER=gles2
mkdir -p /tmp/swaycfg && printf 'exec sleep infinity\n' > /tmp/swaycfg/config
sway -c /tmp/swaycfg/config &
```

Two traps that cost time and are not obvious:

* **`weston --tty=1` is fatal on Weston 14** — `unhandled option`, printed
  *after* EGL has already initialised successfully, so the log reads like a
  graphics failure and is not.
* **`seatd-launch` refuses to start** when a seatd is already running
  (`Socket file found at socket path /run/seatd.sock`). Use the running one, or
  `LIBSEAT_BACKEND=builtin`.

## What it prints when it works

```
Loading module '/usr/lib/libweston-14/gl-renderer.so'
EGL version: 1.5 Bifrost-"g24p0-00eac0"
EGL vendor: ARM
EGL client APIs: OpenGL_ES
EGL features:  EGL Wayland extension: yes
GL renderer: Mali-G52
GL ES 3.2 - renderer features:  OES_EGL_image_external: yes
Using GL renderer
Chosen EGL config details: id: 1 rgba: 8 8 8 0 buf: 24 ... XRGB8888
```

and `weston-terminal` connects and draws.

## A dead end worth recording

Dropping **Mesa's** `libgbm` in front of the Mali driver — Debian trixie's
`libgbm1` 25.0.7, arm64 — gets past the missing symbol and then segfaults. GBM
and EGL have to be the same implementation: Mesa's `gbm_create_device` returns a
Mesa device and Mali's EGL reads it as its own. Half a stack is not a stack.

## Do the emulators still work on it?

Yes, as far as a log can say. Each core was run against a real game for a fixed
spell on both drivers, twice over — RetroArch cores first, then the standalone
emulators, which reach the driver directly rather than through RetroArch's GL
layer.

| | stock `g13p0` | `g24p0-wayland-gbm` |
|---|---|---|
| snes9x | ran | ran |
| mgba | 17 s | 17 s |
| pcsx_rearmed | 17 s | 17 s |
| mupen64plus-next | 14 s | 14 s |
| flycast (x3) | 14 s, clean | 14 s, clean |
| flycast standalone | GLES 3.2, Mali-G52 | GLES 3.2, Mali-G52 |

Speed and rendered output are compared below.

Every flycast run on both drivers passed its own `glBlitFramebuffer test
successful` check, which is the emulator probing a GL capability and getting a
working answer rather than merely failing to crash. The standalone build reports
`Vendor 'ARM' Renderer 'Mali-G52' Version 'OpenGL ES 3.2 v1.g24p0-00eac0'` — the
new blob, in use, by name.

Two things not to read into this:

* **One flycast run did segfault on the newer blob** during the first pass. It
  did not reproduce: three further runs on each driver were clean. Recorded
  because it happened, not because it means anything yet.
* **`mupen64plus` standalone fails on both drivers** with `not a valid ROM
  image` on a `.zip`. That is a ROM-format complaint, identical either side, and
  not a driver difference.

### Speed, and the picture

Both of those *can* be measured after all. RetroArch has `--max-frames=N`, which
runs an exact number of frames and exits — so wall-clock is a fair comparison —
and `--max-frames-ss`, which writes the last frame to a PNG. 900 frames, each
core, each driver:

| | stock `g13p0` | `g24p0-wayland-gbm` |
|---|---|---|
| flycast (Dreamcast) | 22,005 ms — ~40 fps | 22,125 ms — ~40 fps |
| pcsx_rearmed | 16,593 ms — ~54 fps | 16,742 ms — ~53 fps |
| snes9x | 16,279 ms — ~55 fps | 16,313 ms — ~55 fps |

Within half a percent everywhere, which is noise. Flycast is the meaningful one:
at 40 fps it is *not* hitting the vsync ceiling, so that number is the machine's
real limit under load — and it is the same limit on both drivers.

And the frames those runs wrote out are **byte-identical between the two
drivers**, all three of them, by md5. Not similar: the same file. They are real
pictures too, not black screens — the PSX capture is the Eurocom logo with its
gradient intact, 4,513 distinct colours.

So the newer blob renders the same pixels at the same speed. What no measurement
here covers is *stutter* — a still frame and a frame count cannot show a hitch
every few seconds — so playing something is still the last word before this goes
on permanently.

## Trying it without installing it

`scripts/flip-mali-shim.sh` stages the blob under `/tmp` on the device and
prints the line to run with it. `/usr/lib` is never written to, and a reboot
forgets the whole thing.

Sources: [ROCKNIX/libmali](https://github.com/ROCKNIX/libmali) ·
[Miyoo-Flip-Mainline-Linux-Reverse-Engineering](https://github.com/Zetarancio/Miyoo-Flip-Mainline-Linux-Reverse-Engineering)
(`docs/drivers-and-dts/drivers.md` for the mali_kbase/libmali pairing and the
Panfrost conflict) · [Mesa: panfrost GPU IDs for G52 1-Core-2EE (RK3568/RK3566)](https://www.mail-archive.com/mesa-commit@lists.freedesktop.org/msg116463.html)
