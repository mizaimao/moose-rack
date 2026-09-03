# Outbox — written, not sent

**Written, not sent.** Sending either is Frank's call.

Both of these are finished text addressed to somebody outside this project,
and neither has been sent. They are here so they are not forgotten, not
because they are done.

Sending either is Frank's call.

Was `upstream-romfile-500.md` and `screenscraper-devid-request.md`.


---

## Upstream bug report — draft

*Was `upstream-romfile-500.md`.*

Ready to post to https://github.com/rommapp/romm/issues/new.
Not filed yet: `gh` is not authenticated on this machine, and posting is your call.

Searched first — no existing report covers this endpoint. The closest prior art
is #3635, which fixed the same class of bug (`DetachedInstanceError` from a
lazy load after the session closed) by eager-loading the missing relationship.

---

**Title:** `GET /api/roms/{id}/files` returns 500 for every valid rom file id (`DetachedInstanceError` on `RomFile.rom`)

**Body:**

#### Describe the bug

`GET /api/roms/{id}/files` returns HTTP 500 for any valid `RomFile.id`. The
endpoint appears to be unusable — I could not find an id that works.

Serialising the result into `RomFileSchema` requires `is_top_level`, and
`RomFile.is_top_level` reads `self.rom`:

```python
# backend/models/rom.py:164
def is_top_level(self) -> bool:
    # File is the same as the rom's full path, or nested file in the rom's directory
    return self.rom.full_path == (
        self.file_path if self.is_nested else self.full_path
    )
```

but `get_rom_file_by_id` eager-loads only `track_meta`:

```python
# backend/handler/database/roms_handler.py
def get_rom_file_by_id(self, id: int, session: Session = None) -> RomFile | None:
    return session.scalar(
        select(RomFile)
        .options(selectinload(RomFile.track_meta))
        .filter_by(id=id)
        .limit(1)
    )
```

By the time Pydantic reads `is_top_level`, the session opened by
`@begin_session` has closed, so the `RomFile.rom` lazy load raises.

#### To reproduce

```console
$ curl -s -o /dev/null -w '%{http_code}\n' -u user:pass \
    'http://romm.example/api/roms/9263/files'
500
```

Any existing `RomFile.id` reproduces it; I tried several across different
platforms and both single-file and multi-file roms.

#### Server log

```
pydantic_core._pydantic_core.ValidationError: 1 validation error for RomFileSchema
  Error extracting attribute: DetachedInstanceError: Parent instance
  <RomFile at 0x7f97158c7b50> is not bound to a Session; lazy load operation of
  attribute 'rom' cannot proceed
  (Background on this error at: https://sqlalche.me/e/20/bhk3)
  [type=get_attribute_error, input_value=Nuke Your Mum! (PD).smc (9263 -> 8750),
   input_type=RomFile]
```

#### Expected behavior

The endpoint returns the `RomFileSchema` for that file.

#### Suggested fix

Eager-load the parent alongside `track_meta`, matching what #3635 did:

```python
.options(selectinload(RomFile.track_meta), joinedload(RomFile.rom))
```

`joinedload` rather than `selectinload` since it is a many-to-one — one row,
one extra join, no second query.

The endpoint already fetches the parent rom a few lines later for the
visibility check (`db_rom_handler.get_rom(file.rom_id)`), so an alternative is
to pass that rom into the schema construction instead of re-resolving it
through the relationship.

#### Version

- RomM 5.0.0 (Docker)
- Confirmed still present on `master` at the time of writing: `get_rom_file_by_id`
  loads only `track_meta`, and `RomFileSchema` still requires `is_top_level`.

#### Workaround

`GET /api/roms/{id}?with_files=true` returns the same per-file data (including
`md5_hash`/`sha1_hash`) and works fine — that path evidently loads the
relationship. Note the two take different ids: `/files` takes a `RomFile.id`,
this takes a `Rom.id`.

---

### Why this matters here

moose-rack verifies folder ROMs per member file, and needs each member's
md5. It uses the `with_files=true` workaround above rather than this endpoint,
so nothing is blocked — but the workaround is the reason we care that the
documented route is broken.


---

## ScreenScraper developer credentials — request draft

*Was `screenscraper-devid-request.md`.*

Post in the **WebAPI** section of the ScreenScraper forum:
<https://www.screenscraper.fr/forumsujets.php?frub=12>

API documentation, for reference: <https://www.screenscraper.fr/webapi2.php>

You need an ordinary screenscraper.fr account first; the credentials are issued
to the *software*, not to the person, which is why an account alone cannot call
the API.

Their forum is French-speaking, so a French version is below the English one.
Post whichever you prefer — or both, English underneath.

---

### English

**Subject:** devid request — moose-rack (open source RomM client)

> Hello,
>
> I would like to request API credentials for **moose-rack**, an open source
> desktop client for self-hosted [RomM](https://romm.app) game libraries. It runs
> on macOS, Windows and Linux, and launches games in RetroArch from a gamepad.
>
> Source: <https://github.com/mizaimao/moose-rack>
>
> I would use the API to scrape metadata and media for the user's own library —
> box art, cartridge, miximage, marquee, screenshots — stored in the ES-DE media
> layout so it stays interchangeable with ES-DE itself.
>
> Each installation is one user, signing in with their own ScreenScraper
> account, and the client is single-threaded with a delay between requests. It
> respects the thread allowance of the account it is using and does not
> parallelise beyond it.
>
> softname: `moose-rack`
>
> Thank you for the work you put into the database.

---

### Français

**Sujet :** demande de devid — moose-rack (client RomM open source)

> Bonjour,
>
> Je souhaite demander des identifiants API pour **moose-rack**, un client
> de bureau open source pour les bibliothèques de jeux auto-hébergées
> [RomM](https://romm.app). Il fonctionne sur macOS, Windows et Linux, et lance
> les jeux dans RetroArch à la manette.
>
> Code source : <https://github.com/mizaimao/moose-rack>
>
> J'utiliserais l'API pour récupérer les métadonnées et les médias de la
> bibliothèque personnelle de l'utilisateur — jaquette, support, miximage,
> marquee, captures d'écran — stockés selon l'arborescence média d'ES-DE afin de
> rester interchangeables avec ES-DE.
>
> Chaque installation correspond à un seul utilisateur, qui se connecte avec son
> propre compte ScreenScraper. Le client est mono-thread avec une pause entre
> les requêtes, et respecte le nombre de threads autorisé par le compte utilisé.
>
> softname : `moose-rack`
>
> Merci pour le travail accompli sur la base de données.

---

### What to do with the credentials

They go in `config.toml` under `[scraper]`:

```toml
ssid = "your screenscraper login"
sspassword = "your screenscraper password"
devid = "issued to you"
devpassword = "issued to you"
softname = "moose-rack"
max_threads = 1
```

`max_threads` is not decoration. ScreenScraper allocates simultaneous
connections by account tier and answers an exceeded allowance with a rejection
rather than a picture, so a client that ignores it scrapes nothing and looks
broken while doing it.

Until the credentials arrive, the app scrapes through the RomM server's own
ScreenScraper account instead — see `src/scrape.rs` for why that route is
legitimate and what it costs.
