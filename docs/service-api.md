# The library service

**Current.** Built, running, and serving every route the app calls.

`src-service/` is the thing that replaces RomM. It answers the endpoints
`src/api.rs` already calls, so pointing the app at it is a change of
`[server] url` and nothing else — no client code moves, and both can run side by
side and be compared.

    moose-service --root       /home/frank/moose-library/ES-DE \
                  --roms       /home/frank/moose-library/ROMs \
                  --collections /home/frank/moose-library/collections \
                  --firmware   /home/frank/moose-library/bios \
                  --inventory  /home/frank/moose-library/inventory.db

Running on `dev.lan:8001` as a systemd **user** unit with linger on. Port comes
from `MOOSE_SERVICE_PORT` in the unit's `Environment=`, so changing it is one
line and a restart.

## The rules

Read these before adding anything. Each exists because something went wrong
without it.

### The filesystem is the truth; the index is a cache you can delete

Every route is answered from a scan of the ES-DE tree — 6,411 games in 0.1 s on
the server, 11,473 in 2.4 s against the SSD. Nothing is stored that a rescan
cannot rebuild. If you find yourself wanting a migration, you have put state
somewhere it does not belong.

### Row ids are the scan index, and are not durable

`id = position + 1`, one-based. Zero is what a missing field deserializes to, and
a game that silently becomes "game 0" costs an evening.

**Never persist anything against a rom id.** RomM's ids *were* durable and that
is precisely what made a rename a migration: the row, the collection membership,
the save and eleven kinds of artwork all pointed at a number. Identity here is
the content hash, and `inventory.db` holds md5, sha1 and crc32 for every file.

Save ids are the exception and are derived — `md5(rom_id, file_name)` — so they
survive the index being deleted rather than renumbering and invalidating every
device's bookkeeping.

### Say what is true, not what is convenient

`SKIP_HASH_CALCULATION` reports whether an inventory actually loaded. When it is
true the client says "size only" out loud and downloads are unverified — which is
right, but only when it is true. It was hardcoded `true` for one commit and every
transfer in that window was size-checked.

### A missing thing is `[]`, not 404

The client reads a missing endpoint as an error and an empty list as "none yet".
`/api/collections` on a tree with no lists is a legitimate "none".

## The routes

15 of 15 that `src/api.rs` calls.

| route | notes |
| --- | --- |
| `GET /` | a status page, for a person who typed the address. Was a 404, which reads as "server down" |
| `GET /api/heartbeat` | `{"SYSTEM":{"VERSION":…}}`, the crate version |
| `GET /api/config` | excluded files/exts, and `SKIP_HASH_CALCULATION` |
| `GET /api/users/me` | single local user |
| `GET /api/platforms` | derived from the scan, counts included |
| `GET /api/roms` | paged, `limit`/`offset`, `{items,total}` |
| `GET /api/roms/identifiers` | every id. **Its absence is a 400, not a 404** — see below |
| `GET /api/roms/{id}` | one rom |
| `GET /api/roms/{id}/content/{*name}` | bytes, **Range-capable**. The name is ignored; the id decides |
| `GET /api/collections` | the curated text lists |
| `GET /api/firmware` | BIOS tree, flattened, hashes included |
| `GET /api/firmware/{id}/content/{*name}` | one BIOS file |
| `POST /api/devices` | register, or return the id already held for that name |
| `GET /api/saves` | optional `rom_id` |
| `POST /api/saves` | multipart `saveFile`, **409 on conflict** |
| `GET /api/saves/{id}/content` | one save's bytes |
| `POST /api/sync/negotiate` | the plan |
| `POST /api/sync/sessions/{id}/complete` | closes a session |

Artwork is mounted twice on the same directory:
`/assets/romm/resources/esde-media` because `media.rs` builds that path itself
from a hardcoded constant, and `/assets/media` for when that constant retires.
Both, so the change can happen without a flag day.

## Traps, each of which cost something

**A route that does not exist answers 400, not 404.** Without
`/api/roms/identifiers`, `/api/roms/{id}` matched, the literal `"identifiers"`
reached the `i64` extractor, and the client silently skipped pruning — its only
way to notice a deletion. Route *order* is irrelevant: axum prefers a static
segment to a dynamic one however they are registered. A commit message here once
claimed otherwise; removing the route fails a test, reordering does not.

**Range is not optional.** The client resumes a part-file with
`Range: bytes=N-` and reads a 200 as "the server ignored me, discard the
partial". Getting it wrong does not fail — it silently re-downloads gigabytes.
`ServeFile` handles it; verify 206 and a correct `content-range`.

**Collections need the platform.** A name alone is not unique across a library
holding both arcade *Contra* and Famicom *Contra*. Un-prefixed, `Arcade Classics`
resolved to famicom games and looked entirely plausible. Lines are
`platform/name    # Display Title` and matching tries the filename first, then
the title — because ES-DE uses the gamelist `<name>` where a scraper filled one
in and the file stem where it did not. Keyed on one alone: 466 of 2,662 one way,
103 the other. On both, with the platform: correct.

**BIOS is not flat.** `bios.rs` wrote every firmware file into one directory on
the reasoning that RetroArch wants it that way. True for RomM's curated
`_retroarch_system`, false for a real `0_BIOS/`: 3,339 files under 2,738 distinct
names, so a sync landed 2,729 and silently overwrote 601. `config.ini` alone
appears 199 times, once per MSX machine. `local_path` reproduces the tree and
trusts neither the reported path nor the name — `..`, `.`, empty and rooted
segments are dropped. `ensure_for_core` stays flat on purpose: it fetches a
*named* BIOS a core asked for.

**Saves are only resolvable when one side moved.** If the device last agreed on
the server's current bytes, it changed here alone → upload. If it last agreed on
the bytes it still holds, the server moved alone → download. Agreed on neither →
**conflict, never resolved silently**. A first sync meeting two different copies
is a conflict too: there is no basis to choose. That per-device bookkeeping is
the whole reason this is a service rather than a network share.

## The app itself, in a browser

`http://dev.lan:8001/` serves `ui/` — the same 12,552 lines the desktop window
runs, unedited. The UI does not speak `/api/`; it speaks Tauri's IPC,
`window.__TAURI__.core.invoke("roms", {...})`, which in a browser is undefined.
Two pieces bridge that, both in `src-service/src/web.rs`:

| route | what it is |
| --- | --- |
| `GET /` | `index.html` with `<script src="/__shim.js">` inserted after `<head>` |
| `GET /settings.html` | the settings page, rewritten the same way |
| `GET /__shim.js` | defines `window.__TAURI__.core` before the app's modules load |
| `POST /invoke/{cmd}` | one IPC command; body is the argument object |
| `GET /media?path=` | one artwork, video or manual, by absolute path |

The desktop app is not a special case of this and this is not a port of it.
Both call the same functions.

### Where a command actually lives

    ui/js/*.js          invoke("roms", {...})
    src-tauri/src/lib.rs    #[tauri::command] fn roms(...) { moose_rack::commands::roms(...) }
    src-service/src/web.rs  "roms" => j!(c::roms(state, p!("platform"), p!("list")))
    src/commands.rs         pub fn roms(state: &AppState, ...)   <- the only implementation

`src/commands.rs` holds 103 functions and `src/app.rs` holds `AppState`, neither
of which mentions Tauri. `AppState::from_config()` is the constructor both
frontends call, so starting the backend does not start a window.

**Do not write a handler in `web.rs`.** A test reads the match arms and fails on
any that does not call `c::`. If a command is missing from `moose_rack`, move it
there and leave a one-line wrapper behind in `src-tauri` — that is what the other
72 look like.

### Adding a command to the web UI

1. If it still lives in `src-tauri/src/lib.rs`, move the body to
   `src/commands.rs`, change `state: State<'_, AppState>` to `state: &AppState`,
   and replace the original with a wrapper that calls it.
2. Add one arm. `j!` serialises a `CmdResult`, `v!` a plain return value, `p!`
   reads an argument by name.
3. Run the tests. `every_command_the_ui_invokes_has_an_arm` scans `ui/js` and
   fails on anything that would answer "unknown command".

### The UI needs a library, and it is the one this service was given

`AppState::from_config_at` builds the state the desktop app holds; the service
then calls `point_at(&layout)` so `roms_dir` and `esde_media` are *this*
service's, whatever `config.toml` said. One source of truth for where the
library is. `media_dir` is left alone -- it is where the app writes art indexes
and fetched icon sets, and an ES-DE tree need not be writable.

`[library] app_config` names that `config.toml`, for the parts of the UI that
are not the library: theme, per-game cores, achievements. Optional. Without it
the defaults apply and the library still lists, because the library did not come
from there.

The UI reads a sqlite metadata cache, not the scan the API answers from, so the
service rescans into it at startup -- `app::scan_into`, the same three calls
`scan-esde` makes. The filesystem is the truth and the cache is derived, which
is the rule the rest of this file already follows. A failure is printed and not
fatal: the API half is unaffected, and a stale list beats a service that will
not start.

**Do not write those three calls out again.** `scan-esde` did, left out
`absorb_local_into_server`, and the CLI's scan sat beside the rows a server sync
had already stored -- 11,062 against 11,473 in a library of about 11,500, every
shared game drawn twice. `both_callers_go_through_scan_into` fails on a direct
`replace_from_esde`.

### Traps in the bridge

**JavaScript sends `localOnly`; Rust wants `local_only`.** The `#[command]`
macro does that rename for the desktop build and nothing does it over HTTP, so
`normalise()` does — on the top-level keys only. Values are left alone, because a
`ListRef` has its own serde names.

**`/media` must not serve by name alone.** Commands answer with absolute paths
because a desktop webview can load a file directly. Over HTTP that same path is a
request for any file on the server. `resolve_media` canonicalises the request
*and* the roots before comparing, which a string prefix does not: with one,
`<media>/../../secret` reads as starting inside the media directory. Roots are
`media_dir`, `themes_dir`, `esde_media` and `theme_root`. Missing and forbidden
answer the same 404, so the route cannot be used to test whether a path exists.

**`ServeFile`, not `read`.** Videos are scrubbed, which is a Range request, and a
200 to one makes the browser refetch the file.

**Opening a window is the browser's job.** `open_settings` and `open_link` are
answered inside the shim with `window.open`. Sent over the wire they would raise
a window on the machine running the service, which is not the machine looking at
the page.

**There is no authentication.** Anyone who can reach the port can read the
library and POST to `/invoke/`, which includes the settings writes. It is a LAN
service on a home network and nothing more.

### What the server refuses, and why

Launching, RetroArch paths, the app's dock icon and the Android hand-off act on
the machine somebody is sitting at. Library sync, BIOS sync, ROM downloads and
scraping copy a library *from* a server *to* local storage, and this is the
server — there is nowhere for them to put anything. All of them answer
`"<cmd> is not available on the server"`, worded differently from
`"unknown command <cmd>"` so a typo does not look like a design limit.

## Adding a route

1. **Write the test first.** `src-service/src/main.rs` has `fn app()` precisely
   so a router can be built without a socket; `built()` gives a two-game ES-DE
   tree on disk.
2. **Check the test fails** when the thing it covers is broken. Two of the
   explanations in this file were wrong until something was deliberately broken
   to see what the test caught.
3. Read the shape the client expects out of `src/api.rs` — it is the contract,
   and it is already written down there.
4. Verify against the running service with the real client, not curl alone. A
   sync found two defects that reading the code did not.

## Where things live on the server

    /home/frank/moose-library/
      ROMs/<system>/                     seeded from the SSD, 7 systems
      ES-DE/gamelists/<system>/gamelist.xml
      ES-DE/downloaded_media/<system>/<type>/
      ES-DE/saves/<rom_id>/<file>        written by the service
      collections/*.txt                  the 27 curated lists
      bios/                              3,339 files
      inventory.db                       59,319 hashes

Note the SSD's own layout differs: it is a *portable* ES-DE install, so its
artwork is under `support/downloaded_media` while its gamelists are not. The
server uses the ordinary arrangement, which `Layout::new` derives with no
override. See `[esde] media` in `config.example.toml`.
