# Controller-first UI: the rules

**Current. Agreed with Frank on 2026-11-01** for every front end from here on,
starting with the Slint prototype (`src-slint/`, branch `slint-prototype`).

The app is driven with a pad. A mouse works and a keyboard works, but every
screen is designed for the pad first and checked with one in hand. "Better"
means these rules hold; each one is written so that it can be tested.

## The rules

1. Nothing needs a pointer. Finding a game, playing it, downloading it,
   starring it, searching, and every setting are reachable with the pad alone.
   Nothing is shown only on hover.
   *Check:* do each of those with the mouse unplugged.

2. One cursor, always visible, never lost. After any action it lands
   somewhere sensible, and closing a screen puts it back where it was. Each
   pane remembers its own position.
   *Check:* open and close every panel and overlay; the cursor ends where it
   started.

3. The D-pad goes where the eye expects. It moves to the nearest thing in
   that direction (`moose_rack::focusring`), never to a dead end, and the edge
   of one pane leads into the next.
   *Check:* from every control, each direction either moves or is visibly the
   edge of the screen.

4. Back always goes up one level, and a few presses from anywhere reach the
   home view. No screen traps the cursor.
   *Check:* from the deepest screen, count the presses home.

5. Buttons keep their jobs everywhere. Confirm is the right face button and
   back the bottom one on every pad (A on Nintendo, Circle on PlayStation, B on
   Xbox). The rest follow Steam's library (Frank, 2026-11-05): the left face
   button (X / Square) filters, the top one (Y / Triangle) sorts, the Menu
   button (Start / Options) opens Options for the game under the cursor, the
   Steam button (Guide; Xbox, PS) opens the main menu, here Settings, and the
   shoulders change section. Triggers jump a page. The hint bar always says
   what the buttons do.
   *Check:* the hint bar on every screen matches what the buttons do.

6. Big lists are quick to cross. Holding a direction speeds up, triggers
   move a page, alphabetical lists can jump by letter, and recent games and
   favorites are near the top.
   *Check:* reach the last game of the biggest platform in a few seconds.

7. As little typing as possible. Filters are choices, not text. Search has
   an on-screen keyboard laid out for a pad.
   *Check:* find a game by name with the pad alone.

8. Readable from the sofa. Sofa mode has bigger type and art, and the
   cursor can be seen across a room.
   *Check:* read it at TV distance.

9. Every press answers at once: the cursor slides, the card lifts, an
   optional menu sound plays, and it all runs at 60 FPS.
   *Check:* no press goes unacknowledged, no frame drops while moving.

When a rule is unclear, do what Steam's Big Picture (the Steam Deck UI) does,
and check it against a source rather than memory (Frank, 2026-11-05: "align to
Steam instead of ES-DE").

## Where the prototype stands (2026-11-04)

| Rule | State |
| --- | --- |
| 1 Nothing needs a pointer | Done, except search typing (rule 7) |
| 2 Cursor never lost | Done: each console and tab keeps its game, clearing a search returns to it, the header keeps its control, Settings its row, and closing Settings goes back to where it opened |
| 3 D-pad goes where expected | Done by zone; not yet through `focusring` |
| 4 Back goes up | Done |
| 5 Fixed button jobs | Done, as in Steam's library: X / Square Filter (only on this device, only favorites), Y / Triangle Sort by (name, year), Menu Options (take offline or remove, star, jump to letter, search, grid or list, random), the Steam button Settings; menus take the pad while open; each hint shows only where its button does something |
| 6 Big lists | Mostly done: a held direction or trigger speeds up to about 30 moves a second (`padpoll::Repeat::fire_accelerating`), the triggers move a screen, and Options has Jump to letter when sorted by name. **Gap:** recently played games are not near the top yet |
| 7 Little typing | Done: with a controller connected, search opens an on-screen keyboard (QWERTY, numbers, ' - : &, then Space, Delete, Clear, Done); the results narrow behind it as each key types, the left face button deletes, the top one types a space, Start is Done. Filters are not built yet |
| 8 Readable from the sofa | Partly: bigger art in Sofa, same type size |
| 9 Every press answers | Mostly done: the grid's cursor is one ring that slides from card to card, the card under it lifts, and every press that does something plays a short tone (a tick to move, rising on Confirm, falling on Back; Settings turns them off). **Gap:** 60 FPS not yet measured on a device; other panes' cursors jump rather than slide |

The gaps are fixed in the order 2, 5, 6, 7, 9. `MOOSE_SLINT_TEST_KEYS` (src-slint/README.md)
plays a list of presses and prints where the cursor lands, to check them.
