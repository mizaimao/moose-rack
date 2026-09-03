# moose-rack — plan and reference

A native desktop client for a self-hosted [RomM](https://romm.app) server. Browse the
library, download ROMs on demand, launch them in RetroArch, and sync saves/states back.

Status: **working client.** Browse, download and launch all function through a
Tauri GUI, a TUI, and a CLI over one shared Rust library. Save sync (§5 stage 5)
is the remaining piece — its `content_hash` gate passes, the scanner works, and
negotiation is not yet wired up.

```
src/            core library — api, cache, download, media, theme, retroarch
src/main.rs     CLI (clap) + TUI frontend
src-tauri/      Tauri GUI shell (thin; delegates to the library)
ui/js/          static ES modules, no bundler
tools/          one-shot extraction scripts
```

GUI keyboard: arrows move, Enter opens/plays, Esc/Backspace goes back, `/`
searches, `?` lists what is bound. Every binding is configurable in Settings
and persisted; download / grid-list / info-pane / themes ship **unbound**,
since bare letters are easy to press by accident.

---

## 1. Why this exists

RomM's in-browser player (EmulatorJS) picks its WASM core based on whether
`SharedArrayBuffer` is available, which requires cross-origin isolation **and a secure
context**. Over plain `http://dev.lan` there is no secure context, so the single-threaded
core is always chosen. Worse, `ppsspp` and `dosbox_pure` ship *thread-only* — they cannot
run in-browser at all. That's 109 PSP ROMs unplayable.

A native client sidesteps this entirely by running a real emulator.

**Honest scope note:** browsing + downloading + launching largely reproduces what ES-DE
already does. The part that genuinely cannot be done any other way — including by pointing
ES-DE at an SMB share — is **save/state sync with real conflict detection across devices**.
That is stage 5, and it is the point of the project.

---

## 2. Environment

| | |
|---|---|
| Server | RomM 5.0.0, `rommapp/romm:latest` + `mariadb:11`, compose at `/home/frank/romm` |
| URL | `http://dev.lan` (port 80 → container 8080), plain HTTP |
| Library | 9,856 ROMs / 36 platforms / ~183 GB |
| Metadata | LibRetro only; IGDB/ScreenScraper/Moby/etc. all disabled |
| Local dev machine | macOS (Apple Silicon), APFS |

`openapi.json` (154 endpoints) is checked in at the repo root. **Treat it as the source of
truth** over any documentation, including this file.

### Local test library

`library/` — 24 platforms, 239 games, 248 ROM files, 1,955 media files. Built by
`tools/build_test_library.py` from an ES-DE collection using APFS copy-on-write clones.

Laid out as RomM "struct_a" (`roms/<platform_slug>/`) so a RomM instance can scan it
directly, and so slugs match the server. `library/manifest.json` records every pick, the
seed, and the ES-DE→RomM mapping.

Verified: 24/24 slugs resolve against the live server, 239/239 games resolve to a `rom_id`,
0 broken `.m3u` references, sampled files MD5-identical to source.

> **The source collection (`~/Downloads/roms_pre_compress`) has been deleted.** The builder
> cannot re-run. `library/` is gitignored and not reproducible without that tree.

---

## 3. Verified API findings

Measured against the live server, not assumed. These override the original handoff notes
where they disagree.

### ⚠ The real latency: a stale DNS record

**`dev.lan` resolves to two A records, and the first one is dead.**

| address | ping | TCP connect |
|---|---|---|
| `10.10.10.199` | 100% packet loss | **75 s, times out** |
| `10.10.10.77` | 4 ms | 0.00 s |

Every new connection tries `.199` first, waits out the TCP timeout, then falls back to
`.77`. Measured repeatedly: `http://dev.lan` = **75.2 s** for any first request on a
connection; `http://10.10.10.77` = **0.20 s**. It affects *everything*, including the
unauthenticated `/api/heartbeat` — it is not RomM, not auth, not the API.

Effect on this client, same code, only the host changed:

| | via `dev.lan` | via `10.10.10.77` |
|---|---|---|
| full sync (9,856 roms) | 83.5 s | **8.5 s** |
| incremental sync | 75.7 s | **0.8 s** |

> **Proper fix is to remove the stale `10.10.10.199` record from LAN DNS.** Until then
> `config.toml` points at the IP. This is very likely a large part of the "latency" that
> motivated building a native client in the first place — worth re-checking whether the web
> UI still feels slow once DNS is fixed.

### Performance — metadata is not slow

Measured against `10.10.10.77` (i.e. with the DNS problem bypassed):

| Operation | Result |
|---|---|
| `/api/roms` paging, 500/page | 0.27–0.56 s per page |
| Full cold pull, all 9,856 ROMs | **~8.5 seconds** |
| Incremental (`updated_after`, nothing changed) | 0.8 s, 0 rows |
| ROM download, 110 MB `.chd` | ~76 MB/s |
| ROM download, 1.5 GB `.chd` | **~115 MB/s** |

> **The earlier "3.8 MB/s" figure was wrong — it was ~95% DNS timeout.** That test moved
> 300 MB in 78.5 s; subtract the 75 s dead-record stall and it is 300 MB in ~3.5 s, i.e.
> ~86 MB/s. Consistent with the numbers above. Transfers run at LAN speed.

An earlier estimate of "43 minutes for a full pull" was **wrong**, and the first explanation
for *why* it was wrong was also incomplete. It was originally attributed to missing
keep-alive. The true cause is the dead DNS record above: that script opened a new TCP
connection per request, so it paid the ~60–75 s timeout roughly 40 times. Keep-alive helped
only because it paid the penalty once instead of per request.

**Consequence:** nothing about this server is slow. Metadata is ~8 s cold and sub-second
incrementally; ROM transfer runs at wire speed. A 1.5 GB PSP image takes ~50 s, not the
~20 minutes predicted from the bad measurement.

This removes the last *performance* argument for the client. Local caching is for offline
use and instant navigation, not speed. **The remaining justification is unchanged and
still sound:** native emulation (in-browser is stuck on single-threaded cores, and PSP
cannot run there at all) and save/state sync with real conflict detection across devices.

> Large-file throughput is one sample; the small file may have been served from the
> server's page cache. Confirm whether the ceiling is network or server disk before
> designing around 3.8 MB/s.

### Gotchas

- **`platform_ids` is an array param, not `platform_id`.** Unknown params are silently
  ignored, so a typo quietly pages the entire library.
- **`/api/roms/{id}/files` returns HTTP 500 for every id tested** — both `rom_id` and a
  valid `romfile_id`. Use `GET /api/roms?with_files=true` instead, which works.
- **Multi-disc games are broken server-side.** RomM indexed only the `.m3u` playlists, never
  the disc images. `Final Fantasy VII (USA)` is rom 8669, **141 bytes**. Searching the whole
  library for `"(Disc"` returns 9 hits, none of them a disc image. Root cause: RomM scans
  only the top level of each platform dir, and ES-DE's shared `MultiDisk/` folder is not
  RomM's expected multi-file layout (RomM wants a directory per game — hence
  `/api/roms/{id}/convert-to-folder`). Affects ~19 games across psx/gc/dreamcast/3do.
  **The client must detect tiny `.m3u` ROMs and refuse them with a clear message.**

### Archive hashing — RomM hashes a composite, not the file

Read out of the running server (`handler/filesystem/roms_handler.py` via SSH),
not inferred. For any archive, RomM streams the decompressed bytes of **every**
eligible member, in ASCII order of the internal path, through one running
digest:

```python
for name, size, chunks in ARCHIVE_READERS[rom_ext](...):
    for chunk in chunks:
        md5_h.update(chunk)      # one hash across all members
```

Members are skipped when the basename matches `DEFAULT_EXCLUDED_FILES`
(`.DS_Store`, `.localized`, `.Trashes`, `.stfolder`, `@SynoResource`,
`gamelist.xml`, `metadata.pegasus.txt`) or the extension is in
`DEFAULT_EXCLUDED_EXTENSIONS` (`db ini tmp bak lock log cache crdownload`).

This explains what looked like broken metadata: a **single**-member zip appears
to match "the file inside" because with one member the concatenation *is* that
member, while multi-member romsets match neither the container nor any member.
`download::hash_archive_composite` reproduces it — verified 10/10 against
arcade romsets — so archive downloads are strictly verified again.

### Identification: prefer hashes over filenames

The server stores `crc_hash`, `md5_hash`, `sha1_hash`, and `ra_hash` per ROM, and
`GET /api/roms/by-hash` round-trips correctly on all three. Use it instead of `fs_name`
matching, which is fragile because this library has `nes`+`famicom`, `snes`+`sfc`, and
`arcade`+`mame` as separate platforms with overlapping names.

### Auth

Three paths exist. `POST /api/auth/device/init` works (returns 201 + `user_code`).

> **Scope trap.** The existing `personal_token` on this server lacks `roms.read` and
> `platforms.read`. A client using it authenticates fine and then gets 403 on `/api/roms`
> and `/api/platforms` — which presents as "pairing failed." This is the most likely reason
> Freegosy could not connect. Any token must include both.

Minimum scopes for this client:

```
me.read  roms.read  platforms.read  assets.read  assets.write
devices.read  devices.write
```

`client_device_identifier` establishes device identity, and the server keys its per-device
sync watermark on the returned `device_id`. **Changing it resets sync history**, turning
quiet no-ops into a full re-upload. Set once, persist forever.

---

## 4. Stack decision — Rust + Tauri

The workload is ~100% I/O bound (HTTP, hashing, file moves), so language *speed* is
irrelevant. Chosen for packaging and UI control:

- **UI feel.** Tauri renders in the system webview (WKWebView on macOS), so the look is
  fully controllable via HTML/CSS. Flutter paints its own widgets and reads slightly off on
  desktop — a rejected option after trying Freegosy.
- **Packaging.** ~10 MB bundles with a documented macOS signing/notarization path. A
  PySide6 equivalent means PyInstaller, 150 MB+ bundles, and a painful notarization story.
- **The required Rust is the easy kind:** `reqwest` + `tokio` + `std::fs`.

The prototype is **TUI-only** (`ratatui` + `crossterm`). No images, no web UI yet.

**Crates:** `reqwest`, `tokio`, `serde`/`serde_json`, `toml`, `ratatui`, `crossterm`,
`rusqlite`, `md-5`, `sha1`, `directories`, `anyhow`.

---

## 5. Staged build plan

Ordered by risk, not by dependency. RetroArch launching is the biggest unknown, so it goes
first — and it can be tested entirely against `library/` with no network.

### Stage 0 — Launch spike ✅ DONE

`src/retroarch.rs` + `src/main.rs`. Confirmed: a SNES game boots and the program detects
exit cleanly. macOS `.app` invocation works by running the inner
`Contents/MacOS/RetroArch` binary directly — do **not** use `open -a`, which returns
immediately instead of blocking.

**Windowed is the default; `--fullscreen` opts in.** Taking over the display uninvited is
obnoxious, and RetroArch's own config can decide otherwise.

Also handled: platform inference from path (including stepping out of `MultiDisk/`),
fallback to an installed alternative core when the default is absent, and refusing an
incomplete `.m3u` before it reaches the emulator.

### Stage 1 — API client + auth ✅ DONE

`src/api.rs` + `src/config.rs`. `cargo run -- check` prints
`frank (id 1, role admin)`, `9856` ROMs, and 24 populated platforms.

One shared `reqwest::Client` gives connection pooling for free — the 300× that the bad
43-minute measurement was missing. `platform_ids` is sent as an array, with a comment
saying why.

### Stage 2 — Library browse (TUI) ✅ DONE

`src/cache.rs` (SQLite) + `src/tui.rs` (ratatui/crossterm).

```
cargo run -- sync [--full]     8.5 s cold, 0.8 s incremental
cargo run -- browse            platforms -> games, launch with Enter
```

Cache holds all 9,856 roms in a 2 MB SQLite file; browsing does zero network I/O.
Incremental sync stores a watermark of `max(updated_at)` rather than "now", so clock skew
between client and server can't silently drop rows.

The browser marks `●` platforms with an installed core and `▣` ROMs present in `library/`.
Enter launches (windowed); the TUI leaves the alternate screen before spawning and restores
after, or the emulator and the TUI fight over the terminal.

**Dependency note:** `rusqlite` is pinned to 0.37 — 0.40 pulls `libsqlite3-sys` 0.38, which
uses the unstable `cfg_select` feature and will not build on stable Rust 1.94.

### Stage 3 — Core mapping ✅ DONE (ahead of schedule)

`data/esde-core-map.json` covers all 24 platforms; see §7. Against the cores already
installed on this machine, **16 of 24 platforms can launch today**:

```
ready:   arcade dc gamegear gb gba gbc mame mastersystem megadrive
         n64 neo-geo-pocket neogeoaes pcengine psx sfc snes
missing: 3do(opera) famicom+nes(mesen) nds(melondsds) ngc(dolphin)
         psp(ppsspp) wonderswan+wonderswancolor(mednafen_wswan)
```

Six unique cores were missing: `opera`, `mesen`, `melondsds`, `dolphin`, `ppsspp`,
`mednafen_wswan`. **All six installed** via `cargo run -- cores --install` (`src/cores.rs`),
which fetches `<stem>_libretro.dylib.zip` from the buildbot and unzips into
`<portable>/cores`. Now **24 of 24 platforms launch**, 39 cores installed.

Note: `dolphin` *is* built for macOS arm64, contrary to older reports of it being x86_64
only.

### Stage 4 — Download + cache

Streaming, resumable via HTTP Range (`HEAD` is supported), verified against the server's
`md5_hash`/`sha1_hash`.

**Done when:** pick a remote game → download → verify → launch, and a killed transfer
resumes rather than restarting.

Verified: interrupting a 1.5 GB transfer at 305 MB and re-running resumes from exactly
that offset and still passes md5 — so a resumed file is byte-correct. Deliberately
planting a corrupt `.part` is caught by the hash and refused rather than renamed into
place. Tiny `.m3u` stubs are rejected before any transfer.

#### Folder ROMs verify per member, not as a whole

A folder ROM's own `md5_hash` **cannot be reproduced by any client**, and this is not a
limitation worth working around — it is a property of how the server computes it.
`roms_handler.py` streams the folder's files through one digest in the order
`utils/filesystem.iter_files` yields them, and that function uses `os.walk`:

```python
for root, _, files in os.walk(path, topdown=True):
    for file in files:
        yield Path(root), file
```

`os.walk` returns whatever order the filesystem hands back — for `snes/Aftermarket` that
is neither sorted nor creation order. Reproducing it would mean reproducing the server's
directory layout, and the value would change if the files were ever rewritten.

The way out is that **each member carries its own hash**. `GET /api/roms/{id}?with_files=true`
returns `md5_hash`/`sha1_hash` per file, so every unpacked file is checked individually.
That is strictly better than a composite: it says *which* file is wrong, not merely that
something is.

Two rules matter when checking members:

- A member that is itself an archive is hashed by its **contents**, not its bytes — the
  same rule as the top-level single-file path, applied one level down. Half the members of
  `snes/Aftermarket` are `.zip`, and the raw-bytes hash matches none of them.
- RomM synthesises an `.m3u` playlist *into the download zip* that does not exist on the
  server. It is not in the file list and must not be counted as an unverified member.
  (Playlists we created server-side for multi-disc games *are* listed, and do get checked.)

Verified end to end — `nes/Battle City` 26 files, `nes/Multicarts` 22, `snes/Aftermarket`
13, `gba/Aftermarket` 25, `psx/Heart of Darkness (USA)` 3 (704 MB): all report
*all N files md5-checked*. Corrupting one member causes a refetch instead of the previous
"already downloaded and verified", which was a false claim — the old code only tested that
the directory was non-empty.

Where the server lists no hash for some member, the count says so
(`13 of 14 files md5-checked`) rather than reporting a clean bill of health.

### Stage 5 — Save sync

**Hash parity gate: ✅ PASSED.** `src/savehash.rs` ports `compute_content_hash`;
`cargo run -- hash-parity` (`src/parity.rs`) uploads crafted saves, compares against the
`content_hash` the *server* computed, then deletes them again. All 5 cases pass:

| case | what it exercises |
|---|---|
| plain binary | raw-MD5 path |
| empty file | streaming-loop boundary |
| zip, entries written out of order | `sorted()` actually applied |
| zip with directory entries | directory members skipped |
| zip, empty member + non-ASCII names | UTF-8 ordering, zero-length member |

Rust orders `str` by UTF-8 bytes where Python orders by code point; these agree because
UTF-8 preserves code-point order. The non-ASCII case confirms that empirically rather than
by argument.

Getting this wrong would mean `no_op` never fires and the entire save set re-uploads on
every run, forever. RomM itself shipped that bug.

```python
# Byte-exact port target. sorted() and the "\n" join are load-bearing.
def compute_content_hash(path):
    if zipfile.is_zipfile(path):
        with zipfile.ZipFile(path, "r") as zf:
            parts = []
            for name in sorted(zf.namelist()):
                if not name.endswith("/"):
                    parts.append(f"{name}:{hashlib.md5(zf.read(name)).hexdigest()}")
            return hashlib.md5("\n".join(parts).encode()).hexdigest()
    h = hashlib.md5()
    with open(path, "rb") as f:
        while chunk := f.read(8192):
            h.update(chunk)
    return h.hexdigest()
```

Then: scan saves → resolve `rom_id` (via `by-hash`) → `POST /api/sync/negotiate` → print the
operation table. **Live on `--dry-run` for several real sessions before writing anything.**

The server performs a genuine three-way merge against a per-device `last_synced_at`
watermark and reports `upload` / `download` / `conflict` / `no_op`. It does **not** resolve
conflicts. Client policy: never auto-overwrite on `conflict`; preserve the loser as
`<name>.conflict-<ISO8601>` and surface it.

Slots pair on `(rom_id, slot)`. A **null slot always negotiates as `upload`**, so unstable
or null slot names cause unbounded duplicate accumulation. Derive deterministically:
`.srm` → `autosave`, `.state1`/`.state2` → `slot1`/`slot2`.

---

## 6. RetroArch integration reference

### Config: `portable.txt` — macOS only

**Verified against RetroArch source (`master`, read 2026-07-30), not documentation.**
Two files implement it; the docs never mention it.

**`file_path_special.c`, `fill_pathname_application_data()`, `#elif defined(OSX)`:**

```c
/* get the directory containing the app */
bundle_url  = CFBundleCopyBundleURL(bundle);
parent_url  = CFURLCreateCopyDeletingLastPathComponent(NULL, bundle_url);
...
/* if portable.txt exists next to the app then we use that directory */
fill_pathname_join(portable_buf, s, "portable.txt", sizeof(portable_buf));
if (path_is_valid(portable_buf))
   return true;
/* if the app itself says it's portable we obey that as well */
   ... CFBundleGetValueForInfoDictionaryKey(bundle, "RAPortableInstall") ...
/* otherwise we use ~/Library/Application Support/RetroArch */
```

So on macOS:

> **`portable.txt` goes in the directory *containing* `RetroArch.app` — beside the bundle,
> not inside it, and not in Application Support.**

```
SomeFolder/
├── RetroArch.app
├── portable.txt        <-- empty file; its existence is the whole signal
├── config/  cores/  saves/  states/  system/  assets/ ...
```

`RAPortableInstall = true` in the bundle's `Info.plist` does the same thing without a file.
A Steam build (`HAVE_STEAM`) is unconditionally portable.

**`frontend/drivers/platform_darwin.m`** then checks portability a second time and, when
set, points the *user documents* tree at the same directory instead of
`~/Documents/RetroArch`. Resulting layout, all relative to the portable dir:

| from `documents_dir_buf` | from `application_data` |
|---|---|
| `saves/` `states/` `system/` | `config/` `config/remaps/` `assets/` |
| `screenshots/` `playlists/` `logs/` | `autoconfig/` `cht/` `database/rdb/` `downloads/` |
| `records/` `records_config/` | |

> **Cores are the exception — their location is build-dependent.** With
> `HAVE_UPDATE_CORES` (the official build, which has the Online Updater) it's
> `<portable>/cores`; with `HAVE_APPLE_STORE` it's `RetroArch.app/Contents/Frameworks`;
> otherwise `RetroArch.app/modules` — *inside* the bundle.

**Resolved on this machine.** RetroArch 1.20.0 is already installed in portable mode:

```
/Users/frank/Data/Games/Emulators/RetroArch/
├── RetroArch.app/Contents/MacOS/RetroArch      (the binary)
├── portable.txt                                 (0 bytes — the signal)
├── cores/          33 dylibs   <-- this build uses <portable>/cores
├── config/  saves/  states/  system/  assets/  autoconfig/  cht/
├── database/  downloads/  info/  logs/  overlays/  playlists/
└── shaders/  thumbnails/
```

`RAPortableInstall` is `false` in `Info.plist`, so `portable.txt` alone is doing the work —
which confirms the mechanism end to end. **Do not hardcode `/Applications/RetroArch.app`;
locate the binary from config or by search.**

### Linux: `portable.txt` does NOT work — use `XDG_CONFIG_HOME`

**Correction to an earlier assumption.** `portable.txt` is macOS-only. `platform_unix.c`
contains **zero** references to it, and the Unix branch of
`fill_pathname_application_data()` does no such check:

```c
#elif !defined(RARCH_CONSOLE)
   const char *xdg = getenv("XDG_CONFIG_HOME");
   if (xdg) { fill_pathname_join(s, xdg, "retroarch/", len); return true; }
   /* else $HOME/.config/retroarch/ */
```

The equivalent lever on Linux is the **`XDG_CONFIG_HOME` environment variable**, set on the
child process when we spawn RetroArch:

```sh
XDG_CONFIG_HOME=<our dir> retroarch -c <ours>/retroarch.cfg -L <core>.so <rom> -f
```

This is arguably *better* than `portable.txt`: it is per-launch and per-process, so it
cannot disturb an existing RetroArch install at all.

(There is also a compile-time `RARCH_UNIX_CWD_ENV` that makes RetroArch use `getcwd()`, but
that requires a custom build and is not usable with a stock binary.)

Windows appears to work differently again — the `_WIN32` branch only reads `%APPDATA%`, with
no `portable.txt` check in either file. Not investigated further; out of scope for now.

### Linux: plain binary, not AppImage

**Decision stands, but for a different reason than originally recorded.** It is *not*
because we need to drop `portable.txt` beside the executable — that mechanism doesn't exist
on Linux. It is because a read-only AppImage bundle blocks placing a prepared config/core
tree next to the binary, and a plain extracted install keeps that option open. Note that
`XDG_CONFIG_HOME` would work with an AppImage too, so this is a preference, not a hard
requirement.

### Always pass `--config` as well

Independent of portable mode, pass an explicit config so behavior never depends on what
RetroArch happens to find:

```
retroarch -c <ours>/retroarch.cfg -L <core>.dylib <rom> -f
```

Relevant `retroarch.cfg` keys:

| key | purpose |
|---|---|
| `libretro_directory` | where cores live |
| `system_directory` | BIOS / firmware |
| `savefile_directory` | `.srm` saves |
| `savestate_directory` | save states |
| `screenshot_directory` | screenshots |

Setting a value to `"default"` makes it relative to the content directory.

### Command-line flags worth knowing

| flag | meaning |
|---|---|
| `-L`, `--libretro` | core to load |
| `-c`, `--config` | use this config file |
| `--appendconfig` | overlay extra configs (later wins) |
| `-f`, `--fullscreen` | force fullscreen |
| `-s`, `--save` | **override savefile path for this launch** |
| `-S`, `--savestate` | **override savestate path for this launch** |
| `-v`, `--verbose` | verbose logging |

`-s` and `-S` are significant for stage 5: we can redirect RetroArch's save output into a
directory we manage, per launch, without modifying any global config.

### Downloading cores automatically

Cores come from the libretro buildbot as zipped dylibs:

```
https://buildbot.libretro.com/nightly/apple/osx/arm64/latest/<core>_libretro.dylib.zip
https://buildbot.libretro.com/nightly/apple/osx/x86_64/latest/...
https://buildbot.libretro.com/nightly/apple/ios-arm64/latest/...
```

400+ cores, most rebuilt nightly. Download + unzip is all that's required — **easy**, an
afternoon of work.

Sizes for reference: `gambatte` 387 KB, `mgba` 427 KB, `snes9x` 685 KB,
`genesis_plus_gx` 864 KB, `mesen` 1.1 MB, `swanstation` 1.5 MB, `mupen64plus_next` 2.3 MB,
`ppsspp` 6.7 MB, `fbneo` 13.9 MB.

> Not every core exists for `arm64` (Dolphin has been missing there). Fall back to
> `x86_64` under Rosetta, or report the gap.

### Downloading RetroArch itself

Harder than cores, but tractable: the macOS build ships as a `.dmg`, so it's
`hdiutil attach` → copy the `.app` → `hdiutil detach`. Freegosy does exactly this (it
handles `.zip`, `.7z`, `.dmg`, `.tar.gz`, `.tar.xz`, `.AppImage`).

macOS gotcha: downloaded files get a `com.apple.quarantine` xattr and Gatekeeper may block
programmatic launch. May need `xattr -d com.apple.quarantine`. **Unverified.**

Default macOS paths (confirm on the machine):

```
/Applications/RetroArch.app/Contents/MacOS/RetroArch
~/Library/Application Support/RetroArch/config/retroarch.cfg
~/Library/Application Support/RetroArch/cores/*.dylib
```

---

## 7. Core map (extracted — done)

`data/esde-core-map.json` — generated by `tools/extract_esde_cores.py`. **All 24 RomM
platforms in `library/` have a default core, and every one of them was verified present on
the libretro macOS arm64 buildbot (2026-07-30).**

| RomM platform | core | | RomM platform | core |
|---|---|---|---|---|
| 3do | `opera` | | nds | `melondsds` |
| arcade | `mame` | | neo-geo-pocket | `mednafen_ngp` |
| dc | `flycast` | | neogeoaes | `fbneo` |
| famicom | `mesen` | | nes | `mesen` |
| gamegear | `genesis_plus_gx` | | ngc | `dolphin` |
| gb | `gambatte` | | pcengine | `mednafen_pce` |
| gba | `mgba` | | psp | `ppsspp` |
| gbc | `gambatte` | | psx | `mednafen_psx` |
| mame | `mame` | | sfc | `snes9x` |
| mastersystem | `genesis_plus_gx` | | snes | `snes9x` |
| megadrive | `genesis_plus_gx` | | wonderswan | `mednafen_wswan` |
| n64 | `mupen64plus_next` | | wonderswancolor | `mednafen_wswan` |

The JSON also carries every *alternative* emulator per system in ES-DE's preference order,
so "launch with a different core" (a later goal) needs no new research.

**Two cores are named differently on Android than on desktop** — encoded as
`ANDROID_CORE_ALIASES` in the extractor:

```
mamearcade             -> mame                 (Android names current MAME "mamearcade")
mupen64plus_next_gles3 -> mupen64plus_next     (GLES3 variant is Android-only)
```

Filename pattern is otherwise identical across platforms — only the suffix changes:

```
<stem>_libretro.dylib   macOS
<stem>_libretro.so      Linux
<stem>_libretro.dll     Windows
<stem>_libretro_android.so   Android
```

### What the device export did and didn't contain

The three files pulled off the handheld (`es_systems.xml`, `es_find_rules.xml`,
`es_settings.xml`) turned out to cover **less than expected**:

- `es_systems.xml` was the **custom_systems override** file — 15 ROM-hack variants (`nesh`,
  `snesh`, `gbh`, `genh`, `msu-md`…) and modern consoles (`switch`, `ps2`, `ps3`, `psvita`,
  `n3ds`), *not* ES-DE's full system list. It covered only 9 of the 24 platforms.
- `es_find_rules.xml` was **100% Android standalone emulator packages** (AetherSX2, Citra
  variants, Yuzu/Suyu/Citron, RPCSX, Vita3K). Zero libretro cores; no value off Android.
- `es_settings.xml` contained no per-system emulator choices. It sets
  `AlternativeEmulatorPerGame=true`, which means **per-game emulator overrides are stored in
  the `gamelist.xml` files**, and those were not exported. If those per-game choices matter
  later, that is what to pull.

The remaining 15 platforms came from ES-DE's upstream bundled list, vendored at
`data/vendor/esde_android_es_systems.xml` (195 systems) so the extraction stays reproducible
after the device export is deleted.

## 8. Aligning with the Android ES-DE setup

The Android handheld's ES-DE install already encodes a working platform→core mapping. Two
separate things to get from it:

**The default mapping is not personal** — it's ES-DE's bundled
`resources/systems/android/es_systems.xml`, identical for every user. Just read it from the
[ES-DE repo](https://gitlab.com/es-de/emulationstation-de/-/blob/master/resources/systems/android/es_systems.xml)
rather than extracting from the device. Entries look like:

```xml
<command label="mGBA">%EMULATOR_RETROARCH% %EXTRA_CONFIGFILE%=...
  %EXTRA_LIBRETRO%=%INTERNALDATA%/%ANDROIDPACKAGE%/cores/mgba_libretro_android.so
  %EXTRA_ROM%=%ROM%</command>
<command label="Mesen">... /cores/mesen_libretro_android.so ...</command>
```

**Core names translate directly between platforms.** Strip `_android.so`, append `.dylib`:

```
mesen_libretro_android.so   ->  mesen_libretro.dylib
mgba_libretro_android.so    ->  mgba_libretro.dylib
```

So the Android mapping transfers to macOS unchanged. This is the whole alignment story.

**What *is* personal** is any per-system "alternative emulator" you selected, which ES-DE
records in its own settings rather than in `es_systems.xml`. To capture that, pull the
config folder over ADB:

```sh
adb devices                                   # confirm the handheld is visible
adb pull /storage/emulated/0/ES-DE/settings/  ./esde-android/
adb pull /storage/emulated/0/ES-DE/custom_systems/ ./esde-android/
```

Skip `downloaded_media/` and `gamelists/` — large and not needed for core mapping.
Requires USB debugging enabled. **Exact filenames under `settings/` are unverified** —
list the directory first.

Also useful: ES-DE writes a `systeminfo.txt` into every ROM folder listing that system's
full launch commands and core names, and `ES-DE/logs/es_log.txt` records which emulators
were detected at startup.

---

## 9. Prior art

Checked July 2026. No first-party desktop client does save sync.

| Project | Stack | Platforms | State | Save sync |
|---|---|---|---|---|
| [Freegosy](https://github.com/abduznik/Freegosy) | Flutter/Dart | macOS, Win, Linux | 157★, v0.5.10 | ✓ saves only |
| [rommapp/grout](https://github.com/rommapp/grout) | Go | Linux handhelds | 186★, first-party | ✓ |
| [gameflow-deck](https://github.com/simeonradivoev/gameflow-deck) | Bun + React + webview | Win, Linux (Mac untested) | 70★ | experimental |
| [romm-retroarch-sync](https://github.com/Covin90/romm-retroarch-sync) | Python | Linux AppImage | 98★, v1.0.6 | ✓ saves + states |
| [romm-client](https://github.com/chaun14/romm-client) | Electron/TS | Desktop | 33★, self-declared POC | ✓ |
| [romm-esde-bridge](https://github.com/Mrzhao2018/romm-esde-bridge) | Python | SteamOS, Win, Linux | 1 commit, 0★ | read-only |

**Freegosy** was evaluated and rejected: it could not pair with this server (almost
certainly the token scope trap in §3), and its Flutter UI was not wanted. It downloads
emulator binaries at runtime rather than bundling them, and launches them as **separate
external processes** — not embedded.

---

## 10. Design decisions taken

- **Launch emulators as external processes, fullscreen.** In-window emulation is only
  possible via libretro cores loaded in-process (a large project in its own right, with
  immature Rust bindings), and is impossible for standalone emulators like Dolphin or PCSX2
  — macOS has no equivalent of X11's XEmbed. Fullscreen external launch feels nearly
  identical for a fraction of the work and covers every emulator.
- **Do not embed RomM's web UI in a webview.** That reintroduces the single-threaded
  EmulatorJS problem the project exists to escape.
- **RetroArch only for the prototype.** Standalone Dolphin / PCSX2 / PPSSPP each need their
  own save layout; defer.
- **Cache ROMs, not metadata-first.** Metadata is 8 s cold; ROM transfer is the real cost.

## 11. Storage layout (decided)

All downloaded data lives in one plain folder beside the executable:

```
<project root>/library/
├── roms/<platform>/<game>          downloaded ROMs
└── downloaded_media/<platform>/    covers, screenshots, videos
    ├── covers/     fetched from the server, or imported from ES-DE
    ├── screenshots/
    └── videos/     ES-DE only — the server has no videos
```

**Explicitly not `~/Library/Application Support/Moose Rack`**: hard to find, easy to
forget, and it accumulates gigabytes invisibly. A visible folder can be
inspected, backed up, or deleted wholesale, and nothing in it is unrecoverable.

The GUI shows the absolute paths and total size in the status line (hover for
detail) so it is never a mystery where the disk went.

Artwork resolution prefers local ES-DE media and falls back to the server,
caching fetched files into the *same* ES-DE-shaped tree — so imported and
fetched art are interchangeable and there is only one lookup path. Local media
covers ~2% of the library; the server has covers for **92%** and screenshots
for 89%.

## 12. Console icons — ES-DE themes

`src/theme.rs` (local) + `src/theme_remote.rs` (downloader).

```
themes                            list installed themes + logo coverage
themes --install                  copy logos into library/downloaded_media/_platforms/
themes --available [filter]       list the 64 themes in the official ES-DE list
themes --get <name>               clone one (depth 1)
themes --get <name> --logos-only  clone, keep the 24 icons, delete the rest
themes --update                   update everything downloaded
```

**Deliberately not RomM's `url_logo`** — it points at `images.igdb.com` (32 of 34
platforms) and `cdn.thegamesdb.net` (2), i.e. third-party CDNs, not the RomM server.
ES-DE themes are local, offline, SVG, and match the frontend already in use.

Themes disagree on where logos live; three conventions are checked directly and
anything else is found by a depth-limited sweep for `<system>.<ext>` inside a
directory whose name contains "logo":

| convention | example theme |
|---|---|
| `<theme>/<system>/images/logo.svg` | slate-es-de |
| `<theme>/system/logos/<system>.svg` | linear-es-de |
| `<theme>/_inc/system-logo/<system>.svg` | canvas-es-de |

Discovery order puts downloaded themes first, then `~/ES-DE/themes`, then the
ES-DE.app bundle, and falls through theme by theme per platform — so a sparse
custom theme still contributes what it has.

> **`--logos-only` matters.** Canvas is a 298 MB checkout of which we render
> 240 KB. Cloning, extracting, and deleting keeps the icons without the bulk.

Slug→system naming needs care: ROM-hack systems (`genh`, `msu-md`) claim the
same RomM slug as their base system, so every candidate name is tried rather
than picking one alphabetically.

## 13. Open questions

- Whether save **states** negotiate through `/api/sync/negotiate` — the payload only
  declares a `saves` array. Unverified.
- `overwrite` / `autocleanup` / `autocleanup_limit` behavior on `POST /api/saves`.
- Token refresh/expiry — `expires_at` is returned, no refresh endpoint spotted. The
  existing `personal_token` expires 2026-08-29.
- `/api/saves/{id}/track`, `/untrack`, `/downloaded`, `/visibility` — purpose unknown.
Settled since:

- ~~Arcade core compatibility.~~ **Resolved.** The three arcade platforms are three
  different romset vintages and want different cores: `arcade`→fbneo, `mame`→mame2003_plus,
  `neogeoaes`→fbneo. Measuring them as one thing is why every earlier attempt contradicted
  itself. 97% of 3,295 games now launch; see [docs/arcade-coverage.md](docs/arcade-coverage.md).
  Method: CRC32 comparison against each core's DAT, validated 14/14 against real launches.
  Two traps: romset *name* matching proves nothing (MAME knows `sfiii` and still refuses
  it — the set needs a CHD), and requiring a full BIOS set marks every Neo Geo game broken
  when only one of its many regional BIOSes is needed.

- ~~Multi-disc: server-side or client-side?~~ **Server-side.** 18 games were restructured
  into per-game folders and rescanned; the 18 stale `.m3u` stubs were deleted. The client
  gained cache pruning, zip unpacking and a platform-inference fix alongside.
- ~~Is 3.8 MB/s on large files a network or server-disk ceiling?~~ **Neither** — it was
  ~95% dead-DNS timeout (§3). Actual throughput is ~115 MB/s.

## 14. Platform support

macOS, Windows and Linux. What actually differs, and where:

| | macOS | Windows | Linux |
|---|---|---|---|
| RetroArch binary | `RetroArch.app/Contents/MacOS/RetroArch` | `retroarch.exe` in root | `bin/retroarch` or on the prefix |
| Core suffix | `.dylib` | `.dll` | `.so` |
| `portable.txt` | honoured | honoured | **ignored by RetroArch** (§6) |
| Data root | `~/Library/Application Support/RetroArch` | `%APPDATA%\\RetroArch` | `$XDG_CONFIG_HOME/retroarch` |
| Cores when not portable | data root `/cores` | data root `\\cores` | also `/usr/lib/*/libretro` |
| Shaders | data root `/shaders` | data root `\\shaders` | also `/usr/share/libretro/shaders` |
| Install helper | mounts the `.dmg` | extracts the `.7z` | refuses, and names the package options |

`portable.txt` being Linux-irrelevant is not an oversight — RetroArch genuinely
ignores it there and always follows XDG, so the flag is never even read.

**TLS backend is `rustls` + `ring`, deliberately.** reqwest's default pulls
`aws-lc-sys`, a large C library whose build script demands a full cross
toolchain; it makes a Windows or Linux build fail on a machine that only has a
host compiler. `ring` is lighter and every HTTP client is built through
`util::http_client`, which installs the provider once — reqwest panics if
nothing does.

**Cross-compiling still needs a target C compiler** (`x86_64-linux-gnu-gcc`,
`x86_64-w64-mingw32-gcc`) because `ring` has C and assembly. Build on the
target platform or in CI; that is the normal arrangement, not a defect. The
platform-specific branches themselves are verified to type-check for all three
targets.

## 15. Collections — mirrored, not invented

We are a RomM client, so collections are the server's groupings rather than a local
feature. `sync` replaces the whole set each time; nothing is merged, because virtual
collection ids are derived from name+type and a rename would silently orphan the old row.

Three families, one shape (`id, name, rom_ids, rom_count, is_favorite`), so one table
holds all of them:

| family | endpoint | on this server |
|---|---|---|
| hand-made | `/api/collections` | 0 |
| smart (saved filter) | `/api/collections/smart` | 0 |
| virtual (auto-grouped) | `/api/collections/virtual?type=…` | **1,920** |

Virtual breaks down as company 1,040 · series 511 · franchise 307 · genre 55 · mode 7.
The whole set is ~8 MB and ~46,600 memberships, fetched in about 2 seconds — no reason
to be selective.

Details worth remembering:

- **Ids are not integers.** Hand-made collections use a number, virtual ones a base64
  blob of `{"name":…, "type":…}`. Stored as TEXT so both share a table. This is also why
  collection cards use `data-cid` in the UI: the keyboard code treats any `data-id` as a
  ROM id and would call `Number()` on base64.
- **Kinds are discovered, not hardcoded** — read from `/collections/virtual/identifiers`,
  so a RomM that grows a new kind appears without a client change. The identifiers are
  unpadded base64 in both alphabets; decoding has to tolerate that.
- **Counts come from the join, not the server's `rom_count`.** The two disagree once a
  member ROM is deleted, and a collection that opens empty is worse than one that is
  absent — so empty ones are hidden entirely.
- **Card art reuses the local cover cache** (`rom_covers` on a few member ids) rather
  than the server's `path_covers_small`. Hotlinking would break offline browsing, and
  `/assets` being unauthenticated is not something to build on.
- Collection cards must **escape the light-mode invert** applied to ES-DE logos — that
  filter exists for white-on-transparent SVGs and destroys real cover art.

## 16. Known upstream bug

`GET /api/roms/{id}/files` returns 500 for every valid `RomFile.id` on 5.0.0 and on
`master`: serialising `RomFileSchema` needs `is_top_level`, which lazy-loads `RomFile.rom`
after the session has closed → `DetachedInstanceError`. `get_rom_file_by_id` eager-loads
only `track_meta`.

We do not depend on it — `GET /api/roms/{id}?with_files=true` returns the same per-file
data and works. Note the differing ids: `/files` takes a `RomFile.id`, the other a `Rom.id`.

A write-up ready to post is in [`docs/upstream-romfile-500.md`](docs/upstream-romfile-500.md).
Not filed: `gh` is not authenticated here, and posting is Frank's call.

## 17. Settings — what is done and what is not

The Settings panel is now its own resizable window with a tab rail on the left:
General, Appearance, Control, Library, Emulators, About. It replaced a single
scrolling column that put emulator paths, sync buttons and both binding tables
in one list, so the two longest things in the app sat between the user and
everything else.

Library and Emulators were split out later, for the same reason and one level
down: the things that go and fetch something need to report while they work,
and at the bottom of General under six unrelated headings the BIOS control was
simply never found. "Systems" was renamed to Emulators because every tab in
there is about systems of one kind or another and the word gave no clue that
this is where you choose which core runs a console.

### Stretch goal: a controller picture

Bindings are still a table of rows. The intended version is a drawing of a
controller with each binding attached to the button it belongs to — click the
button on the picture, press the button on the pad. That is how every emulator
frontend worth using does it, and it removes the translation step where someone
has to work out which physical button "Confirm" currently is.

Not done. It needs artwork per controller family (Xbox, DualSense, Switch Pro,
8BitDo) and a hit-mapped overlay, and `padprofile.rs` already knows which pad is
connected, so the picture could follow the hardware. Logged rather than started.

### Still to do

* One field per `config.toml` key. Several settings — shaders, per-game cores,
  the ES-DE paths, achievements — are still config-file only, and the tabs exist
  precisely so they have somewhere to go.
* The Control tab is the two old binding tables moved across unchanged. They now
  sit in a two-column grid, which is an improvement in density and nothing else.

## 18. Bulk download — what it has to ask before it starts

Parked, to be built with the "grab this for the trip" button. The button takes a
platform or a collection; this section is about what else it takes.

Artwork is not one thing and the sizes are not close. A game's static art is
tens of kilobytes per type; its video is tens of megabytes. Fetching everything
for a 2,400-game collection is two orders of magnitude more transfer than
fetching what is actually shown, and nearly all of it would never be looked at.
So the download asks, with these defaults:

* **Artwork** — on, and *minimal* by default: the one image the game list is
  set to draw, plus the miximage the info pane shows. Two files per game covers
  everything the interface displays without opening anything.
* **Full static set** — off. Every image type ES-DE has: box front and back, 3D
  box, cartridge, screenshot, title screen, marquee, fanart. For someone who
  wants the artwork strip in the info pane populated offline.
* **Videos** — off, and its own checkbox rather than part of "full", because it
  is the one choice that changes the size of the download by a factor of ten.
* **Manuals** — off, own checkbox.

The estimate shown next to the button has to move with the checkboxes. "25.6 GB"
against "31 GB" is the difference between a download that finishes before a
flight and one that does not, and it is not guessable from the game count.

Related: the app already fetches art lazily and caches by kind, so a bulk
download is a warm-up of that same cache, not a second mechanism. Whatever it
fetches, browsing later finds locally.

## 19. The window — one pane, or three columns

The app drew one pane at a time: consoles, then that console's games, with a
detail panel that slid over the list and a Back button to undo each step. Every
screen set the top bar itself — hide Back, show Grid, show the zoom slider
unless we are in list mode, set the title — six or seven imperative lines
repeated in each of six functions. That is not just repetitive. Each copy
states, in code, that this is a single-pane app with a back button, so changing
the shape of the window meant editing every view that had an opinion about it,
and they all did.

`ui/js/shell.js` is the seam. A view now *describes* what it wants — a title,
which buttons, whether the zoom slider means anything here — and hands over its
content by role rather than by element:

* `picker` — the list you choose from. Consoles in Library, collections in the
  others. Its own column when there is one, the main pane when there is not.
* `games` / `primary` — the games.
* `aside` — the detail beside them.

Nothing in `shell.js` knows what a console is, and no view knows what a column
is. That split is what made the second layout an implementation of `enter` and
`paint` rather than a change to any view.

Three columns is the default. In one pane, choosing a console replaces the
screen and Back brings it back; in three columns the consoles stay on the left,
choosing one fills the middle, and there is nothing to go back to — so no Back
button, and the preview is a column rather than something to toggle. Both
columns are draggable and remember their widths. The arrangement is kept in
`localStorage` rather than `config.toml`: it is about this window rather than
about the library, and the settings window has to be able to read it too.

### What the left column costs

It is a list, not a grid, and it took eight reports to actually be one. The
collections are drawn as `.card`, which is right for a page of them and wrong
for a 240px column, and twice the "fix" rearranged the *contents* of the box
while `.card` went on drawing the box itself — background, border, 12px radius,
14px padding, and a hover that lifts it off the page. The third attempt turned
all of that off and set `display: flex` without `flex-direction: row`, which
inherits `column` from `.card`, so the rows were still two lines high. The
lesson is in the tests: assert the box is gone and the line runs across,
against the real stylesheet, rather than describing the arrangement inside it.

### Ordering

Consoles are alphabetical and stay that way — thirty-five of them that never
change is a column you learn the shape of. Collections get an order button
(name, most games, fewest, most downloaded) because they arrive from the server
in size order, there are twenty-seven, and which you want at the top depends on
what you are doing. That order is remembered, unlike the game sort in
`sort.js`, which is deliberately forgotten: the order of a game list is a
question you asked about one console and stop caring about when you leave it.

### Still rough

* Coming back up from a collection's games still uses the one-pane trail.
* Continue playing appears in one pane only.
* The middle column inserts every row: 2,506 for the arcade list. Per-row
  listeners were the first cost and are gone — one delegated listener on the
  container serves all of them and survives a redraw — but the nodes themselves
  remain, and windowing them is the next real step.

## 20. Menus

`ui/js/menu.js` is the app's own right-click menu, because this is an
application and WebKit's offers "Open Image in New Window" for a console icon.

It is worth its own section for one line of it. The menu closed on the next
`pointerdown` anywhere on the window — and pressing a menu item *is* a
pointerdown, so the menu came off the page between the press and the release
and the click was retargeted to whatever was left. Every item in every menu did
nothing: the orders in the column, and Delete on a save state, which is why
that one was reported four times as "not right". Each of those fixes made the
menu *appear* correctly; none of them made choosing from it work.

Two things follow from it. `closeMenu` runs before the chosen item's action, so
anything that can throw in there takes the action with it — its cleanup is a
held function rather than a DOM event for exactly that reason. And a test must
press the way a mouse does, down then up then click, and check the menu
survived the press: jsdom will deliver a click to a node that has been removed
from the document and a browser will not, so `.click()` on its own passes
against an app in which every menu is dead.
