// Game ids this page remembered from before stable ids.
//
// Games used to be numbered by position in a scan, and three things in this
// page's storage kept those numbers: the game the cursor was last on in each
// list (`lastRom`), the games set to play in the window (`playHere`), and
// EmulatorJS's own per-game settings -- control mappings, core options,
// cheats -- which it keys `ejs-<gameId>-<core>-<name>-settings` off the
// `EJS_gameID` the player passes it. The
// numbers now name nothing, so the cursor went back to the top and the choice
// was forgotten. The backend knows which game each old number was once a scan
// or sync has placed it, and this asks it, once per start, for whatever is
// still old.
//
// An old id the backend cannot place is kept only while it still might be
// placed. Once the backend says the migration has settled -- a full pull from
// the server has happened, or nothing is left waiting -- it is dropped, and
// what it pointed at comes from the server again: the list opens at the top,
// the game plays the default way, EmulatorJS starts from its defaults. Nothing
// here ever throws past its caller: a preference that could not be moved is
// not a reason for the page not to load.

/// Anything smaller than this in magnitude is an old id. Mirrors
/// `moose_rack::gameid::FLOOR`.
export const FLOOR = 1 << 24;

export const isLegacy = (id) => Number.isInteger(Number(id)) && Math.abs(Number(id)) < FLOOR;

function read(key, fallback) {
  try {
    return JSON.parse(localStorage.getItem(key) ?? "null") ?? fallback;
  } catch {
    return fallback;
  }
}

/// EmulatorJS's per-game settings keys, as `[key, old id, rest of the key]`.
///
/// Read out of the pinned build rather than guessed: `getLocalStorageKey()` is
/// `"ejs-" + (gameId||1) + "-" + core + "-" + name + "-settings"`.
function emulatorKeys() {
  const out = [];
  try {
    for (let i = 0; i < localStorage.length; i += 1) {
      const key = localStorage.key(i);
      const m = /^ejs-(-?\d+)-(.+-settings)$/.exec(key ?? "");
      if (m && isLegacy(m[1])) out.push([key, Number(m[1]), m[2]]);
    }
  } catch {
    // Storage that cannot be walked has nothing we can move.
  }
  return out;
}

/// Rewrite remembered games onto stable ids. Returns how many moved.
///
/// `invoke` is passed in so this can be tested without a backend, and so the
/// page decides when it runs: before anything reads the remembered cursor.
export async function adoptStableIds(invoke, state = null) {
  const lastRom = read("lastRom", {});
  const playHere = read("playHere", []);
  const emulator = emulatorKeys();
  const old = [
    ...Object.values(lastRom).filter(isLegacy),
    ...(Array.isArray(playHere) ? playHere : []).filter(isLegacy),
    ...emulator.map(([, id]) => id),
  ].map(Number);
  if (!old.length) return 0;

  let answer;
  try {
    answer = await invoke("stable_ids", { ids: [...new Set(old)] });
  } catch {
    return 0;
  }
  const map = answer?.moved;
  if (!map || typeof map !== "object") return 0;
  const settled = answer.settled === true;
  const to = (id) => (isLegacy(id) && map[String(id)] != null ? Number(map[String(id)]) : null);
  // Old, untranslatable, and never going to be: dropped.
  const dead = (id) => isLegacy(id) && to(id) == null && settled;

  let moved = 0;
  for (const [k, v] of Object.entries(lastRom)) {
    const n = to(v);
    if (n != null) {
      lastRom[k] = n;
      moved += 1;
    } else if (dead(v)) {
      delete lastRom[k];
      if (state?.lastRom) delete state.lastRom[k];
      moved += 1;
    }
  }
  const nextPlayHere = [...new Set((Array.isArray(playHere) ? playHere : []).flatMap((id) => {
    const n = to(id);
    if (n != null) {
      moved += 1;
      return [n];
    }
    if (dead(id)) {
      moved += 1;
      return [];
    }
    return [id];
  }))];
  for (const [key, id, rest] of emulator) {
    const n = to(id);
    if (n == null) {
      if (dead(id)) {
        try {
          localStorage.removeItem(key);
          moved += 1;
        } catch {
          // Tried again next start.
        }
      }
      continue;
    }
    try {
      const target = `ejs-${n}-${rest}`;
      // A game already played under its new id keeps those settings.
      if (localStorage.getItem(target) == null) {
        localStorage.setItem(target, localStorage.getItem(key));
      }
      localStorage.removeItem(key);
      moved += 1;
    } catch {
      // Left under the old key; tried again next start.
    }
  }
  if (!moved) return 0;
  try {
    localStorage.setItem("lastRom", JSON.stringify(lastRom));
    localStorage.setItem("playHere", JSON.stringify(nextPlayHere));
  } catch {
    // Not saved is not the same as not moved: this session still has them.
  }
  // `state.js` read `lastRom` at import, before this ran.
  if (state?.lastRom) Object.assign(state.lastRom, lastRom);
  return moved;
}
