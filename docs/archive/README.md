# Archive

Work that is finished or superseded. Nothing here describes how things are
now — read it for *why* something is the way it is, never for *what* is true
today.

Kept rather than deleted because each one records a decision or a measurement
that was expensive to produce and is still the reason something looks the way
it does.

| File | Status | Why it is here |
| --- | --- | --- |
| [wrong-names.md](wrong-names.md) | **Applied.** All 126 renames landed | 126 files across 12 systems whose name did not match their contents, found by hashing against No-Intro. Every one was applied in the 2026-08-28 sync — verified against `../lists/audit-sync-applied.tsv`. It reads like a to-do list and is not one |
| [md-rename-plan.md](md-rename-plan.md) | **Applied.** 90/90 landed | The Mega Drive subset of the above, in table form, plus the upload-verify-delete procedure RomM forced. Its 90 entries are exactly the `megadrive` section of `wrong-names.md` |
| [md-rename-manifest.json](md-rename-manifest.json) | **Applied** | The machine-readable form of that plan — old name, final name, SHA-1, and the staging path used at the time. The staging paths are long gone |
| [port-plan.md](port-plan.md) | **CLOSED 2026-08-24** | The Flip-and-Android port plan. Closed the day it was written, when the six outputs it asked for landed in `handheld-os.md`. The five build schemes it settled are live and now documented in `Cargo.toml` |
| [handheld-frontend.md](handheld-frontend.md) | **Superseded** by `../knulli-addon.md` | The original SDL front-end plan. The answer became `moose-patch`, an addon that patches a stock KNULLI install, rather than a front end that replaces it. Measurements from 2026-08-20 are still sound |

The library rename records above are the *only* copy of what those files used
to be called. If a piece of artwork or a save ever turns up under an old name,
this is where the mapping is.
