// Whether the light-gun notice belongs in front of a launch.
//
// Its own file so it can be tested. Importing `actions.js` pulls in `state.js`,
// which reads `window.__TAURI__.core` at module load, so the decision could not
// be reached from a test without standing up half the app -- and this is a
// decision that badly wanted one.

/// Whether the light-gun notice belongs in front of this launch.
///
/// A predicate rather than a condition inline, because getting it wrong is
/// silent and expensive: the notice is a *modal* and `launch` awaits it, so a
/// dialog nobody answers is a launch that never happens. It opened on every
/// SNES game -- the Super Scope exists, so every SNES game reports a gun -- and
/// a double-click in a browser did nothing at all.
///
/// Every word of it is about the desktop launch planner: the mouse aiming the
/// gun, the gun taking the second controller port. Android already skipped it
/// for that reason and a browser is in exactly the same position.
export function askAboutLightGun({ resolving, skipSync, mobile, native }) {
  if (resolving || skipSync) return false;
  // Not where nothing here can launch a process for you.
  if (mobile || !native) return false;
  return true;
}

