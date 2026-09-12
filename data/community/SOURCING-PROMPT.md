# Prompt for the sourcing agent

Hand the block below to the research agent. It is written to be pasted whole
and to need no context from this repo.

What it produces drops straight into `data/community/raw/`, and
`tools/build_lists.py` votes on it — no conversion step, which is the point of
specifying the file format rather than asking for "a list".

---

You are sourcing published "best games" rankings for a retro handheld's game
library. The library holds several thousand games per console, and a list that
long is not browsable — so the front of each console's list is filled with the
games that multiple published rankings agree are worth playing. Your job is to
collect those rankings. Somebody else does the merging.

## What to produce

**One plain-text file per source per platform.** Never merge two sources into
one file; the merging is a vote and it happens later.

Filename: `<slug>__<source>.txt` — the platform slug exactly as listed below,
two underscores, then a short lowercase tag for the publication
(`timeextension`, `nintendolife`, `metacritic`, `retrododo`).

Contents: two comment lines, then one title per line, **in the order the source
ranked them**, best first.

    # source: https://www.timeextension.com/guides/best-sega-genesis-mega-drive-games-of-all-time
    # how: Time Extension reader votes, top 50
    Sonic the Hedgehog 2
    Streets of Rage 2
    Gunstar Heroes
    Phantasy Star IV

Nothing else in the file. No rank numbers, no scores, no blank-line grouping,
no commentary, no markdown.

## The platforms, and the slug each one must use

| slug | console |
| --- | --- |
| `famicom` | Nintendo Family Computer (Japanese) |
| `nes` | Nintendo Entertainment System (Western) |
| `sfc` | Super Famicom (Japanese) |
| `snes` | Super Nintendo Entertainment System (Western) |
| `gb` | Game Boy |
| `gbc` | Game Boy Color |
| `gba` | Game Boy Advance |
| `n64` | Nintendo 64 |
| `ngc` | Nintendo GameCube |
| `megadrive` | Sega Mega Drive / Genesis |
| `mastersystem` | Sega Master System |
| `gamegear` | Sega Game Gear |
| `pcengine` | NEC PC Engine / TurboGrafx-16 |
| `psx` | Sony PlayStation (the first one) |
| `neo-geo-pocket` | SNK Neo Geo Pocket / Pocket Color — note the hyphens |
| `arcade` | Arcade (MAME/FBNeo-era coin-op) |

**Famicom and NES are separate platforms here, and so are Super Famicom and
SNES.** A Japanese ranking goes in the Japanese slug. The same game appearing
in both is expected and correct, not a duplicate.

## How many, and which to prioritise

The merge counts how many sources named each game: a game named by *every*
source for its platform is "agreed", one named by *more than half* is "most".
With two sources those two tiers are identical and the vote does nothing — so
**three independent sources per platform is the floor, five is better.**

What already exists, so you can aim at the gaps:

    arcade            0 sources   <- nothing at all; highest priority
    neo-geo-pocket    1
    gamegear gb gba gbc megadrive n64 psx sfc    2 each
    famicom nes ngc pcengine snes                3 each
    mastersystem      4

`famicom` and `sfc` already have sources but they are unusable — see the
Japanese-title rule below. Replacements for those two are worth as much as new
ones.

"Independent" means the rankings were arrived at separately. Three sites
republishing the same top-50 is one source wearing three hats; say so in the
manifest if you suspect it.

## Rules, each of which exists because it went wrong before

1. **Every title must actually appear on the page you cite.** Do not write down
   a game you believe belongs on a list. A fabricated list is worse than a
   missing one, because it looks like data and it gets voted on.

2. **Check the page is really about that platform.** Metacritic answers
   `/browse/game/ds/` and `/browse/game/playstation/` with its *all-platform*
   all-time chart — real games, plausible list, and not one of them a DS game.
   That nearly put Red Dead Redemption 2 into a Nintendo DS collection. If a
   list's top entries are not games for the console you asked for, discard the
   whole list and say why.

3. **Titles only.** No region tags, no parentheses, no `(USA)`, no file
   extensions, no subtitle guessing. Write the title as the source prints it.

4. **Japanese-script titles need an English pair, English first,
   separated by ` | `:**

       Chrono Trigger | クロノ・トリガー
       Kirby Super Star | 星のカービィ スーパーデラックス

   The library stores most Japanese games under romanised names, so a
   Japanese-only list matches almost nothing. This applies to any title that
   comes back in Japanese, on any platform.

5. **No series entries, no compilations.** "Samurai Shodown (Series)" is not a
   game. Neither is a Collector's Edition bundle. Drop them.

6. **Do not pad.** If a source lists 12 games, the file has 12 lines. A short
   honest list is fine; 20–100 titles per source is typical.

7. **If a platform has no credible published ranking, say so and move on.**
   Neo Geo Pocket has exactly one good source and that is a finding, not a
   failure.

## Arcade, specifically

Arcade is the one with nothing and the one where "best of" lists are least
consistent, because the pool spans thirty years and several thousand boards.
Prefer rankings that name individual arcade games rather than franchises, and
say in the manifest whether a list covers the whole arcade era or one operator,
decade or genre. Three such lists are worth more than one long one.

## Also hand back a manifest

One markdown table covering every file you produce:

| file | url | method | titles | fetched | caveats |

Caveats is where anything that would mislead the merge goes — a list that mixes
two consoles, a ranking whose top ten you could not verify, a source that looks
derivative of another, a page that has since changed. Write them plainly; they
get carried into the curation notes and re-read months later.
