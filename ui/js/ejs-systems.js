// Which of our platforms EmulatorJS can play, and as what.
//
// Two vocabularies meet here. `coremap` speaks ES-DE system directories
// (`genesis`, `sfc`) and RomM platform slugs (`megadrive`, `neogeoaes`);
// EmulatorJS speaks its own 36-name set (`segaMD`, `snes`) and picks a libretro
// core from that. The core names underneath turn out to be the same words we
// already use -- snes9x, fceumm, mgba, fbneo -- but the *system* names are not,
// and that is the only translation needed.
//
// Extracted from `data/emulator.min.js` in the pinned release rather than
// written from memory. If the pin moves, re-read it.

/// Our platform slug -> the `EJS_core` value that plays it.
export const PLAYABLE = {
  arcade: "arcade",
  famicom: "nes",
  gamegear: "segaGG",
  gb: "gb",
  gba: "gba",
  // Gambatte plays Color; EmulatorJS has no separate name for it.
  gbc: "gb",
  megadrive: "segaMD",
  n64: "n64",
  neogeoaes: "arcade",
  "neo-geo-pocket": "ngp",
  ngp: "ngp",
  nes: "nes",
  pcengine: "pce",
  sfc: "snes",
  snes: "snes",
  wonderswan: "ws",
  wonderswancolor: "ws",
  // Disc systems. They run, and the payload is the reason to think twice --
  // see `HEAVY`, which is about size rather than about support.
  psx: "psx",
  nds: "nds",
  saturn: "segaSaturn",
  // DOSBox Pure. A game is eXoDOS's zip with its DOSBOX.CONF inside; see
  // `OPTIONS` and `THREADED` below, and docs/dos.md.
  dos: "dos",
};

/// EmulatorJS cores that will not start without threads, as `requiresThreads`
/// in the pinned `emulator.min.js` lists them.
///
/// Threads need `SharedArrayBuffer`, and a browser hands that only to a page
/// that is cross-origin isolated *and* a secure context: HTTPS, or localhost.
/// The service sends the isolation headers (`cache_headers` in src-service);
/// whether the page arrived over HTTPS is up to how it is reached.
export const THREADED = new Set(["dos"]);

/// Core options a core needs before its game will start, passed as
/// `EJS_defaultOptions`, which EmulatorJS writes into the core's options file.
export const OPTIONS = {
  // The zip carries a DOSBOX.CONF saying how to start the game. Pure ignores it
  // unless told to read it, and sits at its own start menu instead.
  dos: { dosbox_pure_conf: "inside" },
};

/// Platforms with no browser core at all, and why.
///
/// Not an oversight and not a to-do: there is no Dolphin in WebAssembly, and
/// nothing in EmulatorJS's 36 systems covers either of these. The desktop app
/// plays them.
export const UNSUPPORTED = {
  ngc: "GameCube has no browser core — nothing in EmulatorJS runs it",
  gc: "GameCube has no browser core — nothing in EmulatorJS runs it",
  dc: "Dreamcast has no browser core in this build",
  dreamcast: "Dreamcast has no browser core in this build",
};

/// Platforms the page plays without EmulatorJS, each with what to say where
/// they cannot be played.
///
/// Windows 9x runs in js-dos (`win9x-player.js`), which needs the game's zip
/// and a Windows 98 disk from the library service's `/win9x`. The desktop
/// window has neither, and RetroArch has no core for it, so there the answer
/// is to open the library in a browser. See docs/win9x.md.
export const OWN_PLAYER = {
  windows9x: "Windows 9x games play in a web browser, from the library server",
};

/// Typical bytes per game, measured on the library rather than guessed. Used to
/// warn before a browser downloads a disc image into memory.
///
/// The number that matters is the one on the card in front of you, so this is
/// only a fallback for when the rom's own size is not to hand.
export const HEAVY = { psx: 302e6, saturn: 260e6, nds: 42e6 };

/// Anything at or above this gets asked about first. 64 MB is roughly where a
/// download stops being something you can ignore on a home network, and it
/// leaves every cartridge system well clear.
export const ASK_ABOVE = 64e6;

/// What the Play button should do for this platform.
///
/// `{ core, threads, options }` to play, `{ refuse }` with a sentence to show
/// instead. Size is separate on purpose: a big game is a question, not a
/// refusal.
///
/// `isolated` is whether this page can have threads at all
/// (`globalThis.crossOriginIsolated`). A threaded core on a page that cannot is
/// refused here, with the reason, rather than left to EmulatorJS, whose own
/// message is "Error for site owner".
export function browserPlay(platformSlug, isolated = true) {
  const slug = String(platformSlug ?? "").toLowerCase();
  if (UNSUPPORTED[slug]) return { refuse: UNSUPPORTED[slug] };
  // Not EmulatorJS's, so never offered as "this window" with one of its cores.
  if (OWN_PLAYER[slug]) return { refuse: OWN_PLAYER[slug] };
  const core = PLAYABLE[slug];
  if (!core) return { refuse: `No browser core for ${slug || "this platform"}` };
  const threads = THREADED.has(core);
  if (threads && !isolated) {
    return {
      refuse:
        "This game needs browser threads, which only work over HTTPS or on localhost",
    };
  }
  return { core, threads, options: OPTIONS[core] ?? {} };
}

/// True when this game is big enough to ask about first.
export function shouldWarn(platformSlug, sizeBytes) {
  const n = Number(sizeBytes) || HEAVY[String(platformSlug ?? "").toLowerCase()] || 0;
  return n >= ASK_ABOVE;
}

/// Games told to run in this window rather than in RetroArch.
///
/// Remembered per game, in this browser, and deliberately *not* through
/// `set_game_core`: that writes a libretro core name into `config.toml` and
/// every launch resolves against it, so a pseudo-core in there would be a lie
/// the whole app has to keep reading. This is a choice about which emulator
/// runs the game, not about which core RetroArch should use.
///
/// The automatic fallback -- RetroArch has no core for this system, so the
/// window offers to -- is separate and needs none of this. This is for choosing
/// it when RetroArch *would* have worked.
const PLAY_HERE = "playHere";

function playHereSet() {
  try {
    const raw = localStorage.getItem(PLAY_HERE);
    return new Set(raw ? JSON.parse(raw) : []);
  } catch {
    return new Set();
  }
}

export function playHereWanted(id) {
  return playHereSet().has(Number(id));
}

export function setPlayHere(id, on) {
  const set = playHereSet();
  if (on) set.add(Number(id));
  else set.delete(Number(id));
  try {
    localStorage.setItem(PLAY_HERE, JSON.stringify([...set]));
  } catch {
    // A private window forgets the choice at the end of the session. The launch
    // still does what was asked for this time, which is the part that matters.
  }
  return on;
}

/// The value that stands for "this window" in the core dropdown.
///
/// Not a core name: no core is called this, and `game_cores` never returns it,
/// so it cannot collide with a real one.
export const BROWSER_CHOICE = "__this_window__";
