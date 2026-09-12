# Curation notes

Things about the "best of" sources that are true but not visible in the data,
carried forward so they are not rediscovered the hard way. The per-source URL
and method live in the `# source:` / `# how:` header of each file in `raw/`.

## Japanese titles

`sfc` and `famicom` lists are written in Japanese script; the drive's copies of
the same games are romanised (`Actraiser (Japan) (Translated En)`) and only
about half the library's are in Japanese. So a Japanese-only list matches the
slice of the library that happens to store Japanese names and never matches the
drive at all — about 19%.

The fix is a paired title, **English first**:

    Chrono Trigger | クロノ・トリガー
    Kirby Super Star | 星のカービィ スーパーデラックス
    Dragon Quest V: Hand of the Heavenly Bride | ドラゴンクエストV 天空の花嫁

English or romanised release title on the left, Japanese on the right. **This is
implemented as of 2026-09-11** — `community_favorites.keys_for()` splits on the
bar and a hit on either side counts, and `build_lists`, `collection_status` and
the collection builder all go through it.

Note what it was like in between, because the shape recurs: the sourcing agent
delivered correctly paired lists and every one of them scored **0%**, because
the matcher normalised the whole line into one key that was neither half. A
half-built convention is worse than none — the data was right, the check said
it was worthless, and the obvious reading was that the agent had failed.

Two more layers had to learn the same lesson afterwards. `voted.json` keeps the
full paired line rather than the English half, because throwing the Japanese
away at the vote leaves the collection builder unable to find the game. And
`collection_status` now uses the collection builder's own matcher instead of an
exact key lookup: the library stores `Dragon Quest V - Tenkuu no Hanayome (J)
[T+Eng...]`, which no exact key reaches, and a report stricter than the builder
reports games as missing that the builder would have included. sfc coverage
went 0% → 12% → 35% across those three fixes, on unchanged data.

## One publication is one vote

A source file is a *publication*, not a page. SpotGeeks' Super Famicom article
arrived as seven files — one per genre section, all the same URL — which would
have cast seven of the ten sfc votes. Bitvint's arcade top-100 and its 1980s
top-50 are two articles from one site with heavy overlap by construction. Both
were merged into one file each, best-first, duplicates dropped, with the `how:`
line saying what was merged.

Time Extension's main list plus its genre list was already treated this way;
this is that rule written down.

## Sales rankings are not quality rankings

Two of the four `famicom` sources (`kopenguin`, `nantoka`) rank by units
shipped, not by critical or reader opinion — the manifest that came with them
says so plainly. It shows: the voted list is Dragon Quest-heavy in a way no
best-of list would be. They are kept because Famicom has little else, and
because agreement between a sales chart and a reader poll is real agreement.
Weight them if that ever becomes possible; drop them before adding a fifth
opinion-based source.

## `sfc` is the Japan-only tail, not the Super Famicom

This is the thing that made Super Famicom look broken for months, and it is
about the library rather than the lists.

The `sfc` slug holds pachinko, mahjong, Japan-exclusive releases and
English-translated romhacks — `Cyber Knight (J) [T+Eng1.01_AGTP]`,
`西陣パチンコ物語 2`. Every canonical Super Famicom title is filed under `snes`:
Super Mario World, Mario Kart and Kirby Super Star return **zero** rows for
`sfc` and one or more for `snes`. Same for `famicom` against `nes`.

So a published "best Super Famicom games" ranking matched 29% against `sfc`,
was dropped as if it were a list about another console, and the check was
right on its own terms and wrong about the question. Against `sfc` **and**
`snes` together the same list scores 86%.

`SIBLINGS` in `build_lists.py` widens the verification pool to the machine for
those four slugs. **Only the check widens.** The vote and the collection stay
with the slug the file was filed under, so the Super Famicom collection is
still Super Famicom games — the question the check asks is "is this list about
this machine", and the machine is not the folder.

What this does not fix, and should not: `sfc` coverage sits near a third,
because the games its sources name are mostly filed under `snes`. That is the
honest answer for a collection built from the `sfc` folder. Do not merge the
two platforms to make the number go up.

## Wikipedia's API rate limit answers in prose

Pairing Japanese titles to English goes through ja.wikipedia langlinks. The
first version asked one title at a time, tripped the rate limit, caught the
exception and returned `None` — which the caller read as "no English article
exists". Chrono Trigger and Kirby Super Star were both recorded as unpairable.

Batched 40 titles to a request now, with retries, and an error raises instead
of being swallowed. If a pairing run suddenly reports many titles as having no
English article, suspect this before suspecting Wikipedia.

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
