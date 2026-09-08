// Where the page gets EmulatorJS, and where it gets the game.
//
// The same player runs in two places that answer that differently.
//
// * The **web build** is served by `moose-service`, which serves the vendored
//   unpack at `/emulatorjs/` and the ROM at `/rom?id=`. Relative URLs, one
//   origin, nothing to work out.
// * The **desktop window** has no HTTP server behind it. It reads both off disk
//   through a URI scheme registered by the app -- `moose://localhost/data/...`
//   and `moose://localhost/rom/<id>` -- and on Windows, where WebView2 has no
//   custom schemes, Tauri maps that onto `http://moose.localhost/`. So the base
//   is asked for rather than assumed: `browser_play_base` knows the host rule
//   and also knows whether EmulatorJS was ever fetched.
//
// `null` from that command is a real answer: 296 MB of cores is not in git, so
// a fresh clone has none and playing in the window is simply not offered.

/// The URLs for one side, given what that side reported.
///
/// Pure, so the two shapes can be checked without a browser or a backend.
export function urlsFor({ web, base }) {
  if (web) {
    return {
      available: true,
      desktop: false,
      data: "/emulatorjs/data/",
      rom: (id) => `/rom?id=${encodeURIComponent(id)}`,
    };
  }
  // A string or nothing. A backend that answered with some other shape is one
  // that does not know this command, and treating that as a URL builds
  // `[object Object]data/` and fetches cores from nowhere.
  if (typeof base !== "string" || !base) return { available: false, desktop: true };
  // The trailing slash matters: EmulatorJS concatenates `EJS_pathtodata` rather
  // than resolving it, so a missing one fetches every core one directory up.
  const root = base.endsWith("/") ? base : `${base}/`;
  return {
    available: true,
    desktop: true,
    data: `${root}data/`,
    rom: (id) => `${root}rom/${encodeURIComponent(id)}`,
  };
}

let cached = null;

/// Ask once, and remember. Nothing here changes while the app is running.
export async function ejsUrls(invoke) {
  if (cached) return cached;
  if (globalThis.__MOOSE_WEB) return (cached = urlsFor({ web: true }));
  let base = null;
  try {
    base = await invoke("browser_play_base");
  } catch {
    // An older build with no such command. Not available rather than broken.
    base = null;
  }
  return (cached = urlsFor({ web: false, base }));
}

/// For tests, which run several shapes in one process.
export function forgetEjsUrls() {
  cached = null;
}
