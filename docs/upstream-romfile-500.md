# Upstream bug report — draft

Ready to post to https://github.com/rommapp/romm/issues/new.
Not filed yet: `gh` is not authenticated on this machine, and posting is your call.

Searched first — no existing report covers this endpoint. The closest prior art
is #3635, which fixed the same class of bug (`DetachedInstanceError` from a
lazy load after the session closed) by eager-loading the missing relationship.

---

**Title:** `GET /api/roms/{id}/files` returns 500 for every valid rom file id (`DetachedInstanceError` on `RomFile.rom`)

**Body:**

### Describe the bug

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

### To reproduce

```console
$ curl -s -o /dev/null -w '%{http_code}\n' -u user:pass \
    'http://romm.example/api/roms/9263/files'
500
```

Any existing `RomFile.id` reproduces it; I tried several across different
platforms and both single-file and multi-file roms.

### Server log

```
pydantic_core._pydantic_core.ValidationError: 1 validation error for RomFileSchema
  Error extracting attribute: DetachedInstanceError: Parent instance
  <RomFile at 0x7f97158c7b50> is not bound to a Session; lazy load operation of
  attribute 'rom' cannot proceed
  (Background on this error at: https://sqlalche.me/e/20/bhk3)
  [type=get_attribute_error, input_value=Nuke Your Mum! (PD).smc (9263 -> 8750),
   input_type=RomFile]
```

### Expected behavior

The endpoint returns the `RomFileSchema` for that file.

### Suggested fix

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

### Version

- RomM 5.0.0 (Docker)
- Confirmed still present on `master` at the time of writing: `get_rom_file_by_id`
  loads only `track_meta`, and `RomFileSchema` still requires `is_top_level`.

### Workaround

`GET /api/roms/{id}?with_files=true` returns the same per-file data (including
`md5_hash`/`sha1_hash`) and works fine — that path evidently loads the
relationship. Note the two take different ids: `/files` takes a `RomFile.id`,
this takes a `Rom.id`.

---

## Why this matters here

moose-rack verifies folder ROMs per member file, and needs each member's
md5. It uses the `with_files=true` workaround above rather than this endpoint,
so nothing is blocked — but the workaround is the reason we care that the
documented route is broken.
