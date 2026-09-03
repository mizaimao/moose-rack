
---

## Strict renaming of the 1,559 cosmetically-misnamed files

*Parked 2026-08-27.*

> "deferr again"

**What it is:** 1,559 files whose names disagree with No-Intro only in
formatting. Every one is the correct game, already correctly identified. This
is tidier filenames and nothing else.

**Cost:** 1.9 GB of files, so roughly 3.8 GB of transfer — RomM cannot rename,
so each file must be downloaded and re-uploaded. Spread across gbc 317, gb 291,
megadrive 275, mastersystem 244, gamegear 154, pcengine 90, snes 81, gba 33.

**The shapes, in case only some are worth doing:**

* 1,014 — region or language tags differ (`(Japan)` vs `(Japan) (En)`)
* 193 — the file has no region tag at all and the DAT has one
* 163 — fuller official titles (`Dropzone` -> `Archer Maclean's Dropzone`)
* 127 — a subtitle added or dropped
* 32 — article moved (`Bartman Meets Radioactive Man` -> `Simpsons, The - ...`)
* 30 — spacing (`Rod Land` -> `Rodland`)

**Method, already proven on Mega Drive:** rename on the server's filesystem over
SSH rather than re-uploading — it is faster, it survives non-ASCII names, and it
avoids the upload API's latin-1 header limit that mangled nine Japanese titles.
Move the old files aside rather than deleting, then one scan, then
`cleanup_missing_roms`.

**Not to be confused with** the 126 genuinely wrong names in
[wrong-names.md](wrong-names.md); the 90 Mega Drive ones are fixed, 36 remain.
