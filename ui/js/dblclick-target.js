// Which game a double-click meant.
//
// Its own file so it can be tested. Importing `library.js` pulls in `state.js`,
// which reads `window.__TAURI__.core` at module load, so this decision could
// not be reached from a test without standing up the whole app -- and it is a
// two-line decision that cost a day.

/// Which game a double-click meant.
///
/// `hit` is what the event landed on, and it is very often nothing. A
/// `dblclick` is dispatched on the nearest common ancestor of its two clicks,
/// and the first click redraws the card it landed on -- the selection class,
/// the star, the cover arriving -- so the second lands on a different node and
/// the ancestor is the grid, which carries no `data-id`.
///
/// So the selection is the answer: a double-click means "play the thing I just
/// clicked", and the click before it is what selected it. Reading only `hit`
/// meant the handler returned without a word -- no launch, no error, nothing in
/// the console -- which is the whole of "double-click does nothing".
///
/// Its own function so it can be tested. Reaching it through the DOM needs the
/// whole app standing up, and this is a two-line decision that cost a day.
export function doubleClickTarget(hit, selected) {
  if (hit !== null && hit !== undefined) return hit;
  return selected ?? null;
}
