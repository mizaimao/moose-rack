# Curation notes

Things about the "best of" sources that are true but not visible in the data,
carried forward so they are not rediscovered the hard way. The per-source URL
and method live in the `# source:` / `# how:` header of each file in `raw/`.

## Japanese titles

`sfc` and `famicom` lists exist and are **not usable yet**. They are written in
Japanese script; the drive's copies of the same games are romanised
(`Actraiser (Japan) (Translated En)`) and only about half the library's are in
Japanese. So a Japanese-only list matches the slice of the library that happens
to store Japanese names and never matches the drive at all — about 19%.

The fix is a paired title, **English first**:

    Chrono Trigger | クロノ・トリガー
    Kirby Super Star | 星のカービィ スーパーデラックス
    Dragon Quest V: Hand of the Heavenly Bride | ドラゴンクエストV 天空の花嫁

English or romanised release title on the left, Japanese on the right, and the
matcher tries both sides. Same treatment for any Neo Geo, PC Engine or
WonderSwan title that comes back in Japanese.

## Caveats carried from the external research package

* **WonderSwan lists are mono and Color mixed.** All three cover the family,
  not one machine. Staged for both slugs; the check then dropped mono at
  17–30% and kept Color at 47–55%, which is the honest split — the lists are
  really about the Color. There may be no meaningful mono ranking.
* **Neo Geo AES lists mix AES and MVS arcade titles** (Metal Slug and friends).
  Normal for the platform. Say so if AES-hardware-only is ever wanted.
* **Super Famicom, ranking.net:** 174 of 179 titles were verified against the
  live pages; ranks 1–10 were not. Re-fetch
  <https://ranking.net/rankings/best-superfamicom-games> if it matters.
* **Famicom, ranking.net tail:** "Minecraft" is the literal Japanese title of
  the Famicom Tetris port, and Famicom Mini re-releases appear. Not errors.
* **Neo Geo Pocket has one source and stays that way** — deliberate, not a gap.

## Known unfetchable

* A fourth Famicom source at `game.dancing-doll.com` is JS-rendered; a plain
  fetch returns the page shell only. Needs a real browser session.

## Discarded

The research package shipped 45 saved HTML pages as evidence. Not kept — every
source URL is in the header of its `raw/` file, so any of them can be fetched
again, and the check in `build_lists.py` is what decides whether a list is
trustworthy, not whether a copy of the page was archived.
