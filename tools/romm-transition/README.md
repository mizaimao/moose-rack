# RomM transition tools

**Delete this whole directory when the library service replaces RomM.**
Nothing outside it imports anything in here, so `rm -rf tools/romm-transition`
is the entire removal. That is the point of the directory existing.

These four exist only to keep a RomM server fed. They are not part of what
Moose Rack does — they are what maintaining the thing it is replacing costs.

| File | What it does |
| --- | --- |
| `romm_sync.py` | Brings a RomM server in line with the local ES-DE library. Needs delete-then-upload for renames, because RomM's API cannot rename a rom |
| `upload_rom.py` | Uploads through RomM's chunked upload endpoints. RomM will not take a whole file in one request |
| `copy_from_drive.py` | Diffs an attached drive against both inventories and pushes what is missing, through the same chunked upload |
| `push-media-to-server.sh` | Copies this machine's ES-DE artwork into RomM's `resources/esde-media` |

Everything else under `tools/` and `scripts/` survives RomM. Where those still
said "RomM slug" they now say "platform slug" — the flat `roms/<slug>/` layout
is the library's own shape and outlives the server that first imposed it. The
two scripts that reach into RomM's volume on the server host read
`MOOSE_LIBRARY_ROOT` instead, defaulting to today's path.

See [../../docs/library-service.md](../../docs/library-service.md) for what
replaces them.
